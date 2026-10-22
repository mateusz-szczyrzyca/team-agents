package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"errors"
	"math/big"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"
)

// TestPinnedTLSConfigValidityPeriod checks that expired and future certificates are rejected.
func TestPinnedTLSConfigValidityPeriod(t *testing.T) {
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("generate key: %v", err)
	}

	// 1. Expired certificate
	templateExpired := x509.Certificate{
		SerialNumber: big.NewInt(101),
		Subject:      pkix.Name{CommonName: "expired.example.com"},
		NotBefore:    time.Now().Add(-2 * time.Hour),
		NotAfter:     time.Now().Add(-time.Hour),
	}
	derExpired, err := x509.CreateCertificate(rand.Reader, &templateExpired, &templateExpired, &priv.PublicKey, priv)
	if err != nil {
		t.Fatalf("create expired cert: %v", err)
	}
	certExpired, err := x509.ParseCertificate(derExpired)
	if err != nil {
		t.Fatalf("parse expired cert: %v", err)
	}
	hashExpired := sha256.Sum256(derExpired)
	cfgExpired, errExpired := NewPinnedTLSConfig(hex.EncodeToString(hashExpired[:]))
	if errExpired != nil {
		t.Fatalf("unexpected value error: %v", errExpired)
	}
	connExpired := tls.ConnectionState{PeerCertificates: []*x509.Certificate{certExpired}}
	if err := cfgExpired.VerifyConnection(connExpired); err == nil {
		t.Fatal("expected expired certificate to fail verification, got nil")
	}

	// 2. Future certificate (not yet valid)
	templateFuture := x509.Certificate{
		SerialNumber: big.NewInt(102),
		Subject:      pkix.Name{CommonName: "future.example.com"},
		NotBefore:    time.Now().Add(time.Hour),
		NotAfter:     time.Now().Add(2 * time.Hour),
	}
	derFuture, err := x509.CreateCertificate(rand.Reader, &templateFuture, &templateFuture, &priv.PublicKey, priv)
	if err != nil {
		t.Fatalf("create future cert: %v", err)
	}
	certFuture, err := x509.ParseCertificate(derFuture)
	if err != nil {
		t.Fatalf("parse future cert: %v", err)
	}
	hashFuture := sha256.Sum256(derFuture)
	cfgFuture, errFuture := NewPinnedTLSConfig(hex.EncodeToString(hashFuture[:]))
	if errFuture != nil {
		t.Fatalf("unexpected value error: %v", errFuture)
	}
	connFuture := tls.ConnectionState{PeerCertificates: []*x509.Certificate{certFuture}}
	if err := cfgFuture.VerifyConnection(connFuture); err == nil {
		t.Fatal("expected future certificate to fail verification, got nil")
	}

	// 3. Invalid fingerprint returns typed *ValueError
	_, errBad := NewPinnedTLSConfig("invalid-hex-fingerprint")
	if errBad == nil {
		t.Fatal("expected error for invalid fingerprint")
	}
	var verr *ValueError
	if !errors.As(errBad, &verr) {
		t.Fatalf("expected error wrapping *ValueError, got %T: %v", errBad, errBad)
	}
	if !errors.Is(errBad, ErrInvalidArgument) {
		t.Fatalf("expected ErrInvalidArgument sentinel, got %v", errBad)
	}
}

// TestHubAuthTokenModeMissingAllowedAddr verifies token authentication fails closed
// when an instance has a valid token but no configured allowed source IP address.
func TestHubAuthTokenModeMissingAllowedAddr(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthModeToken)
	auth.Token.Tokens = map[string]string{
		"node-without-addr": "valid-token-123",
	}
	// Note: auth.Token.AllowedAddrs does NOT contain "node-without-addr"

	req := httptest.NewRequest(http.MethodPost, "/v1/call", nil)
	req.TLS = &tls.ConnectionState{}
	req.RemoteAddr = "192.0.2.50:1234"
	req.Header.Set("Authorization", "Bearer valid-token-123")
	req.Header.Set("X-Instance-Name", "node-without-addr")

	_, err := auth.Authenticate(req)
	if err == nil || err.Cause != HubAuthAddressMismatch {
		t.Fatalf("expected HubAuthAddressMismatch when no address is configured, got %v", err)
	}
}

// TestHubAuthConcurrentRegisterRace tests concurrent registrations for the same instance name.
// Exactly one goroutine must succeed and all other concurrent callers must receive HubAuthDuplicateInstanceName.
func TestHubAuthConcurrentRegisterRace(t *testing.T) {
	auth := NewHubAuthenticator(HubAuthModeToken)
	const concurrency = 20
	var wg sync.WaitGroup
	successes := 0
	duplicates := 0
	var mu sync.Mutex

	for i := 0; i < concurrency; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			err := auth.RegisterInstance("contested-node")
			mu.Lock()
			defer mu.Unlock()
			if err == nil {
				successes++
			} else if err.Cause == HubAuthDuplicateInstanceName {
				duplicates++
			}
		}()
	}
	wg.Wait()

	if successes != 1 {
		t.Fatalf("expected exactly 1 successful registration, got %d", successes)
	}
	if duplicates != concurrency-1 {
		t.Fatalf("expected %d duplicate rejections, got %d", concurrency-1, duplicates)
	}
}
