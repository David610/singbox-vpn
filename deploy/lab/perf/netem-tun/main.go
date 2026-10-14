// netem-tun: a minimal userspace substitute for `tc qdisc netem`, for
// kernels that ship without sch_netem. It owns one TUN device inside a
// router network namespace; policy routing sends every forwarded packet
// into the TUN, this program delays/drops it according to a rule table
// keyed by (source prefix, destination prefix), and writes it back into
// the same TUN, from where the kernel forwards it normally.
//
// Order is preserved per rule (a FIFO link): jitter can lengthen a
// packet's delay but never lets it overtake an earlier packet of the same
// rule, unlike netem's default reordering jitter.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"math/rand/v2"
	"net/netip"
	"os"
	"os/signal"
	"sync"
	"sync/atomic"
	"syscall"
	"time"
	"unsafe"
)

type Rule struct {
	Src      string  `json:"src"`
	Dst      string  `json:"dst"`
	DelayMs  float64 `json:"delay_ms"`
	JitterMs float64 `json:"jitter_ms"`
	LossPct  float64 `json:"loss_pct"`
	src, dst netip.Prefix
	q        chan pkt
	last     time.Time
	mu       sync.Mutex
	passed   atomic.Uint64
	dropped  atomic.Uint64
}

type pkt struct {
	buf    []byte
	n      int
	depart time.Time
}

const (
	iffTUN   = 0x0001
	iffNOPI  = 0x1000
	tunSetIF = 0x400454ca
)

func openTUN(name string) (*os.File, error) {
	fd, err := syscall.Open("/dev/net/tun", syscall.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	var ifr [40]byte
	copy(ifr[:16], name)
	*(*uint16)(unsafe.Pointer(&ifr[16])) = iffTUN | iffNOPI
	if _, _, e := syscall.Syscall(syscall.SYS_IOCTL, uintptr(fd), tunSetIF, uintptr(unsafe.Pointer(&ifr[0]))); e != 0 {
		syscall.Close(fd)
		return nil, e
	}
	return os.NewFile(uintptr(fd), name), nil
}

var pool = sync.Pool{New: func() any { b := make([]byte, 65536); return &b }}

func main() {
	dev := flag.String("dev", "tun0", "tun device name")
	rulesPath := flag.String("rules", "", "JSON rule file (array of rules)")
	statsEvery := flag.Duration("stats", 0, "print counters at this interval (0 = only on exit)")
	flag.Parse()
	var rules []*Rule
	if *rulesPath != "" {
		raw, err := os.ReadFile(*rulesPath)
		if err != nil {
			log.Fatal(err)
		}
		if err := json.Unmarshal(raw, &rules); err != nil {
			log.Fatal(err)
		}
	}
	for _, r := range rules {
		var err error
		if r.src, err = netip.ParsePrefix(r.Src); err != nil {
			log.Fatalf("rule src %q: %v", r.Src, err)
		}
		if r.dst, err = netip.ParsePrefix(r.Dst); err != nil {
			log.Fatalf("rule dst %q: %v", r.Dst, err)
		}
		r.q = make(chan pkt, 1<<16)
	}
	// Unmatched traffic: zero delay, zero loss.
	def := &Rule{Src: "0.0.0.0/0", Dst: "0.0.0.0/0", q: make(chan pkt, 1<<16)}
	tun, err := openTUN(*dev)
	if err != nil {
		log.Fatalf("open tun: %v", err)
	}
	all := append(append([]*Rule{}, rules...), def)
	for _, r := range all {
		go writer(tun, r)
	}
	go reader(tun, rules, def)
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	var tick <-chan time.Time
	if *statsEvery > 0 {
		tick = time.Tick(*statsEvery)
	}
	for {
		select {
		case <-tick:
			printStats(all)
		case <-sig:
			printStats(all)
			return
		}
	}
}

func printStats(all []*Rule) {
	for _, r := range all {
		fmt.Fprintf(os.Stderr, "rule %s>%s delay=%.1fms jitter=%.1fms loss=%.2f%%: passed=%d dropped=%d\n",
			r.Src, r.Dst, r.DelayMs, r.JitterMs, r.LossPct, r.passed.Load(), r.dropped.Load())
	}
}

func match(rules []*Rule, def *Rule, b []byte) *Rule {
	if len(b) < 20 || b[0]>>4 != 4 {
		return def
	}
	src := netip.AddrFrom4([4]byte(b[12:16]))
	dst := netip.AddrFrom4([4]byte(b[16:20]))
	for _, r := range rules {
		if r.src.Contains(src) && r.dst.Contains(dst) {
			return r
		}
	}
	return def
}

func reader(tun *os.File, rules []*Rule, def *Rule) {
	for {
		bp := pool.Get().(*[]byte)
		n, err := tun.Read(*bp)
		if err != nil {
			log.Fatalf("tun read: %v", err)
		}
		r := match(rules, def, (*bp)[:n])
		if r.LossPct > 0 && rand.Float64()*100 < r.LossPct {
			r.dropped.Add(1)
			pool.Put(bp)
			continue
		}
		d := time.Duration(r.DelayMs * float64(time.Millisecond))
		if r.JitterMs > 0 {
			d += time.Duration((rand.Float64()*2 - 1) * r.JitterMs * float64(time.Millisecond))
			if d < 0 {
				d = 0
			}
		}
		depart := time.Now().Add(d)
		r.mu.Lock()
		if depart.Before(r.last) {
			depart = r.last // FIFO: never overtake an earlier packet
		}
		r.last = depart
		r.mu.Unlock()
		select {
		case r.q <- pkt{buf: *bp, n: n, depart: depart}:
		default: // queue overflow behaves like a tail drop
			r.dropped.Add(1)
			pool.Put(bp)
		}
	}
}

func writer(tun *os.File, r *Rule) {
	for p := range r.q {
		if wait := time.Until(p.depart); wait > 50*time.Microsecond {
			time.Sleep(wait)
		}
		if _, err := tun.Write(p.buf[:p.n]); err != nil {
			r.dropped.Add(1)
		} else {
			r.passed.Add(1)
		}
		b := p.buf
		pool.Put(&b)
	}
}
