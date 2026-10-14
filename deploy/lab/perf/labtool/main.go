// labtool: the measurement endpoints and probes for the singbox-vpn
// performance lab. Standard library only.
//
//	labtool echo   -listen ADDR                 TCP+UDP echo server
//	labtool decoy  -listen ADDR -name NAME      TLS 1.3 server (REALITY handshake target)
//	labtool dns    -listen ADDR -map name=ip,... -delay D   minimal A-record DNS server
//	labtool bulk   -listen ADDR [-tls-name N]   HTTP(S) server: /bytes/N streams N zero bytes
//	labtool connlat -socks ADDR -target HOST:PORT -n N   cold proxied-connection latency
//	labtool rtt    -socks ADDR -target HOST:PORT -n N -interval D  established-connection RTT
//	labtool udprtt -socks ADDR -target HOST:PORT -n N -fresh   UDP via SOCKS5 UDP ASSOCIATE
package main

import (
	"bufio"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"math/big"
	"net"
	"net/http"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"
)

func main() {
	if len(os.Args) < 2 {
		log.Fatal("usage: labtool <echo|decoy|dns|bulk|connlat|rtt|udprtt> ...")
	}
	cmd, args := os.Args[1], os.Args[2:]
	switch cmd {
	case "echo":
		echo(args)
	case "decoy":
		decoy(args)
	case "dns":
		dnsServer(args)
	case "bulk":
		bulk(args)
	case "connlat":
		connlat(args)
	case "rtt":
		rtt(args)
	case "udprtt":
		udprtt(args)
	default:
		log.Fatalf("unknown subcommand %q", cmd)
	}
}

// ---------- servers ----------

func echo(args []string) {
	fs := flag.NewFlagSet("echo", flag.ExitOnError)
	listen := fs.String("listen", ":7", "")
	fs.Parse(args)
	ln, err := net.Listen("tcp", *listen)
	if err != nil {
		log.Fatal(err)
	}
	pc, err := net.ListenPacket("udp", *listen)
	if err != nil {
		log.Fatal(err)
	}
	go func() {
		buf := make([]byte, 65536)
		for {
			n, addr, err := pc.ReadFrom(buf)
			if err != nil {
				return
			}
			pc.WriteTo(buf[:n], addr)
		}
	}()
	for {
		c, err := ln.Accept()
		if err != nil {
			continue
		}
		go func() {
			defer c.Close()
			if tc, ok := c.(*net.TCPConn); ok {
				tc.SetNoDelay(true)
			}
			io.Copy(c, c)
		}()
	}
}

func selfSigned(name string) tls.Certificate {
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	tmpl := &x509.Certificate{
		SerialNumber: big.NewInt(time.Now().UnixNano()),
		Subject:      pkix.Name{CommonName: name},
		DNSNames:     []string{name},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(24 * time.Hour),
	}
	der, _ := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	return tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}
}

func decoy(args []string) {
	fs := flag.NewFlagSet("decoy", flag.ExitOnError)
	listen := fs.String("listen", ":443", "")
	name := fs.String("name", "decoy.test", "")
	fs.Parse(args)
	cfg := &tls.Config{
		Certificates: []tls.Certificate{selfSigned(*name)},
		MinVersion:   tls.VersionTLS13,
		NextProtos:   []string{"h2", "http/1.1"},
	}
	ln, err := tls.Listen("tcp", *listen, cfg)
	if err != nil {
		log.Fatal(err)
	}
	srv := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		io.WriteString(w, "decoy\n")
	})}
	log.Fatal(srv.Serve(ln))
}

func bulk(args []string) {
	fs := flag.NewFlagSet("bulk", flag.ExitOnError)
	listen := fs.String("listen", ":80", "")
	tlsName := fs.String("tls-name", "", "serve HTTPS with a self-signed certificate for this name")
	fs.Parse(args)
	zero := make([]byte, 256<<10)
	http.HandleFunc("/bytes/", func(w http.ResponseWriter, r *http.Request) {
		n, err := strconv.ParseInt(strings.TrimPrefix(r.URL.Path, "/bytes/"), 10, 64)
		if err != nil || n < 0 {
			http.Error(w, "bad size", 400)
			return
		}
		w.Header().Set("Content-Length", strconv.FormatInt(n, 10))
		w.Header().Set("Content-Type", "application/octet-stream")
		for n > 0 {
			k := int64(len(zero))
			if n < k {
				k = n
			}
			if _, err := w.Write(zero[:k]); err != nil {
				return
			}
			n -= k
		}
	})
	http.HandleFunc("/ok", func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, "ok\n") })
	if *tlsName != "" {
		ln, err := tls.Listen("tcp", *listen, &tls.Config{
			Certificates: []tls.Certificate{selfSigned(*tlsName)},
			MinVersion:   tls.VersionTLS13,
		})
		if err != nil {
			log.Fatal(err)
		}
		log.Fatal(http.Serve(ln, nil))
	}
	log.Fatal(http.ListenAndServe(*listen, nil))
}

// dnsServer answers A queries for a fixed map, NXDOMAIN otherwise, AAAA
// with an empty NOERROR answer. Optional artificial delay models resolver latency.
func dnsServer(args []string) {
	fs := flag.NewFlagSet("dns", flag.ExitOnError)
	listen := fs.String("listen", ":53", "")
	mapping := fs.String("map", "", "name=ip,name=ip")
	delay := fs.Duration("delay", 0, "")
	fs.Parse(args)
	m := map[string]net.IP{}
	for _, kv := range strings.Split(*mapping, ",") {
		if kv == "" {
			continue
		}
		p := strings.SplitN(kv, "=", 2)
		m[strings.ToLower(strings.TrimSuffix(p[0], "."))+"."] = net.ParseIP(p[1]).To4()
	}
	pc, err := net.ListenPacket("udp", *listen)
	if err != nil {
		log.Fatal(err)
	}
	buf := make([]byte, 1500)
	for {
		n, addr, err := pc.ReadFrom(buf)
		if err != nil || n < 12 {
			continue
		}
		q := append([]byte(nil), buf[:n]...)
		go func() {
			if *delay > 0 {
				time.Sleep(*delay)
			}
			if resp := answer(q, m); resp != nil {
				pc.WriteTo(resp, addr)
			}
		}()
	}
}

func answer(q []byte, m map[string]net.IP) []byte {
	// parse QNAME
	i := 12
	var labels []string
	for i < len(q) && q[i] != 0 {
		l := int(q[i])
		if i+1+l > len(q) {
			return nil
		}
		labels = append(labels, string(q[i+1:i+1+l]))
		i += 1 + l
	}
	if i+5 > len(q) {
		return nil
	}
	qtype := binary.BigEndian.Uint16(q[i+1 : i+3])
	qend := i + 5
	name := strings.ToLower(strings.Join(labels, ".")) + "."
	resp := make([]byte, 0, 512)
	resp = append(resp, q[0], q[1]) // id
	flags := uint16(0x8180)         // response, RD, RA, NOERROR
	ip, ok := m[name]
	if !ok {
		flags = 0x8183 // NXDOMAIN
	}
	resp = binary.BigEndian.AppendUint16(resp, flags)
	resp = binary.BigEndian.AppendUint16(resp, 1) // qdcount
	an := uint16(0)
	if ok && qtype == 1 {
		an = 1
	}
	resp = binary.BigEndian.AppendUint16(resp, an)
	ns := uint16(0)
	if an == 0 {
		ns = 1 // RFC 2308: negative answers carry an SOA so resolvers can cache them
	}
	resp = binary.BigEndian.AppendUint16(resp, ns)
	resp = binary.BigEndian.AppendUint16(resp, 0)
	resp = append(resp, q[12:qend]...)
	if an == 1 {
		resp = append(resp, 0xc0, 0x0c)
		resp = binary.BigEndian.AppendUint16(resp, 1)
		resp = binary.BigEndian.AppendUint16(resp, 1)
		resp = binary.BigEndian.AppendUint32(resp, 300)
		resp = binary.BigEndian.AppendUint16(resp, 4)
		resp = append(resp, ip...)
	} else {
		// SOA for the root zone: mname=".", rname=".", serial/refresh/retry/expire, minimum=300
		resp = append(resp, 0x00)                     // owner "."
		resp = binary.BigEndian.AppendUint16(resp, 6) // TYPE SOA
		resp = binary.BigEndian.AppendUint16(resp, 1) // CLASS IN
		resp = binary.BigEndian.AppendUint32(resp, 300)
		rdata := []byte{0x00, 0x00}
		for _, v := range []uint32{1, 3600, 600, 86400, 300} {
			rdata = binary.BigEndian.AppendUint32(rdata, v)
		}
		resp = binary.BigEndian.AppendUint16(resp, uint16(len(rdata)))
		resp = append(resp, rdata...)
	}
	return resp
}

// ---------- SOCKS5 client helpers ----------

func socksConnect(socks, target string, timeout time.Duration) (net.Conn, error) {
	c, err := net.DialTimeout("tcp", socks, timeout)
	if err != nil {
		return nil, err
	}
	c.SetDeadline(time.Now().Add(timeout))
	if _, err := c.Write([]byte{5, 1, 0}); err != nil {
		c.Close()
		return nil, err
	}
	r := make([]byte, 2)
	if _, err := io.ReadFull(c, r); err != nil || r[1] != 0 {
		c.Close()
		return nil, fmt.Errorf("socks auth: %v %v", r, err)
	}
	host, portStr, _ := net.SplitHostPort(target)
	port, _ := strconv.Atoi(portStr)
	req := []byte{5, 1, 0}
	if ip := net.ParseIP(host).To4(); ip != nil {
		req = append(req, 1)
		req = append(req, ip...)
	} else {
		req = append(req, 3, byte(len(host)))
		req = append(req, host...)
	}
	req = binary.BigEndian.AppendUint16(req, uint16(port))
	if _, err := c.Write(req); err != nil {
		c.Close()
		return nil, err
	}
	hdr := make([]byte, 4)
	if _, err := io.ReadFull(c, hdr); err != nil || hdr[1] != 0 {
		c.Close()
		return nil, fmt.Errorf("socks connect reply %v %v", hdr, err)
	}
	var skip int
	switch hdr[3] {
	case 1:
		skip = 4 + 2
	case 4:
		skip = 16 + 2
	case 3:
		l := make([]byte, 1)
		io.ReadFull(c, l)
		skip = int(l[0]) + 2
	}
	if _, err := io.ReadFull(c, make([]byte, skip)); err != nil {
		c.Close()
		return nil, err
	}
	c.SetDeadline(time.Time{})
	return c, nil
}

type result struct {
	Name     string    `json:"name"`
	N        int       `json:"n"`
	Failures int       `json:"failures"`
	P50      float64   `json:"p50_ms"`
	P90      float64   `json:"p90_ms"`
	P95      float64   `json:"p95_ms"`
	P99      float64   `json:"p99_ms"`
	Max      float64   `json:"max_ms"`
	Mean     float64   `json:"mean_ms"`
	Jitter   float64   `json:"jitter_ms"` // mean |delta| between consecutive samples
	Samples  []float64 `json:"-"`
}

func summarize(name string, s []float64, fails int) result {
	r := result{Name: name, N: len(s), Failures: fails, Samples: s}
	if len(s) == 0 {
		return r
	}
	var jit, sum float64
	for i, v := range s {
		sum += v
		if i > 0 {
			d := v - s[i-1]
			if d < 0 {
				d = -d
			}
			jit += d
		}
	}
	r.Mean = sum / float64(len(s))
	if len(s) > 1 {
		r.Jitter = jit / float64(len(s)-1)
	}
	c := append([]float64(nil), s...)
	sort.Float64s(c)
	pct := func(p float64) float64 {
		idx := int(p*float64(len(c)-1) + 0.5)
		return c[idx]
	}
	r.P50, r.P90, r.P95, r.P99, r.Max = pct(.5), pct(.9), pct(.95), pct(.99), c[len(c)-1]
	return r
}

func emit(r result) {
	b, _ := json.Marshal(r)
	fmt.Println(string(b))
}

func ms(d time.Duration) float64 { return float64(d) / float64(time.Millisecond) }

// connlat: n sequential cold proxied TCP connections; each measures SOCKS
// CONNECT -> first echoed byte (the full new-connection cost a client pays).
func connlat(args []string) {
	fs := flag.NewFlagSet("connlat", flag.ExitOnError)
	socks := fs.String("socks", "127.0.0.1:1080", "")
	target := fs.String("target", "", "")
	n := fs.Int("n", 50, "")
	gap := fs.Duration("gap", 20*time.Millisecond, "")
	timeout := fs.Duration("timeout", 10*time.Second, "")
	name := fs.String("name", "connlat", "")
	fs.Parse(args)
	var s []float64
	fails := 0
	for i := 0; i < *n; i++ {
		t0 := time.Now()
		c, err := socksConnect(*socks, *target, *timeout)
		if err == nil {
			c.SetDeadline(time.Now().Add(*timeout))
			_, err = c.Write([]byte{'x'})
			if err == nil {
				_, err = io.ReadFull(c, make([]byte, 1))
			}
			c.Close()
		}
		if err != nil {
			fails++
		} else {
			s = append(s, ms(time.Since(t0)))
		}
		time.Sleep(*gap)
	}
	emit(summarize(*name, s, fails))
}

// rtt: one established proxied connection, n 64-byte ping-pongs.
func rtt(args []string) {
	fs := flag.NewFlagSet("rtt", flag.ExitOnError)
	socks := fs.String("socks", "127.0.0.1:1080", "")
	target := fs.String("target", "", "")
	n := fs.Int("n", 200, "")
	interval := fs.Duration("interval", 20*time.Millisecond, "")
	duration := fs.Duration("duration", 0, "if set, run for this long instead of n samples")
	name := fs.String("name", "rtt", "")
	timeout := fs.Duration("timeout", 10*time.Second, "per-ping deadline; a longer one tells a connection that survived an outage from one that died")
	fs.Parse(args)
	c, err := socksConnect(*socks, *target, 15*time.Second)
	if err != nil {
		emit(result{Name: *name, Failures: 1})
		return
	}
	defer c.Close()
	msg := make([]byte, 64)
	in := make([]byte, 64)
	var s []float64
	fails := 0
	deadline := time.Now().Add(*duration)
	br := bufio.NewReader(c)
	for i := 0; ; i++ {
		if *duration > 0 {
			if time.Now().After(deadline) {
				break
			}
		} else if i >= *n {
			break
		}
		c.SetDeadline(time.Now().Add(*timeout))
		t0 := time.Now()
		if _, err := c.Write(msg); err != nil {
			fails++
			break
		}
		if _, err := io.ReadFull(br, in); err != nil {
			fails++
			break
		}
		s = append(s, ms(time.Since(t0)))
		time.Sleep(*interval)
	}
	emit(summarize(*name, s, fails))
}

// udprtt: SOCKS5 UDP ASSOCIATE; -fresh opens a new association (a new UDP
// session through the tunnel) per sample, otherwise one association is reused.
func udprtt(args []string) {
	fs := flag.NewFlagSet("udprtt", flag.ExitOnError)
	socks := fs.String("socks", "127.0.0.1:1080", "")
	target := fs.String("target", "", "")
	n := fs.Int("n", 50, "")
	fresh := fs.Bool("fresh", false, "")
	interval := fs.Duration("interval", 20*time.Millisecond, "")
	name := fs.String("name", "udprtt", "")
	fs.Parse(args)
	host, portStr, _ := net.SplitHostPort(*target)
	port, _ := strconv.Atoi(portStr)
	hdr := []byte{0, 0, 0, 1}
	hdr = append(hdr, net.ParseIP(host).To4()...)
	hdr = binary.BigEndian.AppendUint16(hdr, uint16(port))
	var s []float64
	fails := 0
	var ctrl net.Conn
	var uc *net.UDPConn
	var relay *net.UDPAddr
	open := func() error {
		if ctrl != nil {
			ctrl.Close()
			uc.Close()
		}
		var err error
		ctrl, err = net.DialTimeout("tcp", *socks, 5*time.Second)
		if err != nil {
			return err
		}
		ctrl.Write([]byte{5, 1, 0})
		r := make([]byte, 2)
		if _, err = io.ReadFull(ctrl, r); err != nil {
			return err
		}
		ctrl.Write([]byte{5, 3, 0, 1, 0, 0, 0, 0, 0, 0})
		rep := make([]byte, 10)
		if _, err = io.ReadFull(ctrl, rep); err != nil || rep[1] != 0 {
			return fmt.Errorf("udp associate failed: %v %v", rep, err)
		}
		relay = &net.UDPAddr{IP: net.IP(rep[4:8]), Port: int(binary.BigEndian.Uint16(rep[8:10]))}
		if relay.IP.IsUnspecified() {
			relay.IP = net.ParseIP(strings.Split(*socks, ":")[0])
		}
		uc, err = net.DialUDP("udp", nil, relay)
		return err
	}
	if !*fresh {
		if err := open(); err != nil {
			emit(result{Name: *name, Failures: *n})
			return
		}
	}
	payload := append(append([]byte(nil), hdr...), make([]byte, 64)...)
	buf := make([]byte, 2048)
	for i := 0; i < *n; i++ {
		t0 := time.Now()
		if *fresh {
			if err := open(); err != nil {
				fails++
				continue
			}
		}
		uc.SetDeadline(time.Now().Add(5 * time.Second))
		if _, err := uc.Write(payload); err != nil {
			fails++
			continue
		}
		if _, err := uc.Read(buf); err != nil {
			fails++
			continue
		}
		s = append(s, ms(time.Since(t0)))
		time.Sleep(*interval)
	}
	if ctrl != nil {
		ctrl.Close()
	}
	emit(summarize(*name, s, fails))
}
