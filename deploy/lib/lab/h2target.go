// h2target: a local TLS 1.3 + HTTP/2 server used by nested_vision_lab.sh both as the REALITY
// handshake decoy and as the traffic target. GET /blob?n=BYTES streams n pseudo-random bytes.
package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"fmt"
	"math/big"
	"net/http"
	"os"
	"strconv"
	"time"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: h2target PORT")
		os.Exit(2)
	}
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	tpl := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "lab"}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(24 * time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, DNSNames: []string{"localhost", "decoy.test"}}
	der, _ := x509.CreateCertificate(rand.Reader, tpl, tpl, &key.PublicKey, key)
	mux := http.NewServeMux()
	mux.HandleFunc("/blob", func(w http.ResponseWriter, r *http.Request) {
		n, _ := strconv.Atoi(r.URL.Query().Get("n"))
		buf := make([]byte, 32*1024)
		rand.Read(buf)
		w.Header().Set("Content-Length", strconv.Itoa(n))
		for n > 0 {
			c := len(buf)
			if n < c {
				c = n
			}
			if _, err := w.Write(buf[:c]); err != nil {
				return
			}
			n -= c
		}
	})
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, "nested-vision-lab-target") })
	srv := &http.Server{Addr: "127.0.0.1:" + os.Args[1], Handler: mux, TLSConfig: &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{{Certificate: [][]byte{der}, PrivateKey: key}}, NextProtos: []string{"h2", "http/1.1"}}}
	fmt.Println("listening", srv.Addr)
	if err := srv.ListenAndServeTLS("", ""); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
