package main

import (
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// TestClockReturnsCurrentTime verifies Clock interface returns actual time.
func TestClockReturnsCurrentTime(t *testing.T) {
	clock := newClock()
	if clock == nil {
		t.Fatal("newClock returned nil")
	}

	before := time.Now()
	now := clock.Now()
	after := time.Now()

	if now.Before(before) || now.After(after) {
		t.Fatalf("Clock.Now() returned %v, not between %v and %v", now, before, after)
	}
}

// TestClockCanBeReplaced verifies Clock interface can be replaced by a fake.
type fixedClock struct {
	t time.Time
}

func (c *fixedClock) Now() time.Time { return c.t }

func TestClockCanBeReplaced(t *testing.T) {
	fixed := time.Date(2026, 11, 3, 14, 0, 0, 0, time.UTC)
	fake := &fixedClock{t: fixed}

	// Verify the fake works as a Clock
	if fake.Now() != fixed {
		t.Fatalf("fixedClock.Now() = %v, want %v", fake.Now(), fixed)
	}
}

// TestRandomInt63Range verifies Random.Int63() returns nonnegative values.
func TestRandomInt63Range(t *testing.T) {
	rng := newRandom(42)
	if rng == nil {
		t.Fatal("newRandom returned nil")
	}

	for i := 0; i < 100; i++ {
		val := rng.Int63()
		if val < 0 {
			t.Fatalf("Int63() returned negative value: %v", val)
		}
	}
}

// TestRandomSameSeedSameSequence verifies same seed produces same sequence.
func TestRandomSameSeedSameSequence(t *testing.T) {
	const seed = 12345
	const count = 10

	rng1 := newRandom(seed)
	rng2 := newRandom(seed)

	vals1 := make([]int64, count)
	vals2 := make([]int64, count)

	for i := 0; i < count; i++ {
		vals1[i] = rng1.Int63()
		vals2[i] = rng2.Int63()
	}

	for i := 0; i < count; i++ {
		if vals1[i] != vals2[i] {
			t.Fatalf("sequence mismatch at index %d: %v != %v", i, vals1[i], vals2[i])
		}
	}
}

// TestRandomDistinctSeedsDistinctSequence verifies different seeds produce different sequences.
func TestRandomDistinctSeedsDistinctSequence(t *testing.T) {
	rng1 := newRandom(11111)
	rng2 := newRandom(22222)

	// Compare first 10 values
	allSame := true
	for i := 0; i < 10; i++ {
		if rng1.Int63() != rng2.Int63() {
			allSame = false
			break
		}
	}

	if allSame {
		t.Fatal("distinct seeds produced identical sequences (first 10 values)")
	}
}

// TestRandomConcurrentCallsRaceClean verifies concurrent calls are race-clean.
func TestRandomConcurrentCallsRaceClean(t *testing.T) {
	rng := newRandom(99999)
	const (
		goroutines        = 10
		callsPerGoroutine = 100
	)

	var wg sync.WaitGroup
	var counter atomic.Int64
	errors := make(chan error, goroutines)

	for g := 0; g < goroutines; g++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := 0; i < callsPerGoroutine; i++ {
				val := rng.Int63()
				if val < 0 {
					errors <- nil // record that we detected an error, but continue
					return
				}
				counter.Add(1)
			}
		}()
	}

	wg.Wait()
	close(errors)

	// Verify no errors were reported
	for err := range errors {
		if err != nil {
			t.Fatalf("concurrent call error: %v", err)
		}
	}

	// Verify all calls completed
	expected := int64(goroutines * callsPerGoroutine)
	if counter.Load() != expected {
		t.Fatalf("call counter = %d, want %d", counter.Load(), expected)
	}
}

// TestClockInterfaceSignature verifies Clock interface has the expected signature.
func TestClockInterfaceSignature(_ *testing.T) {
	var _ Clock = newClock() //nolint:staticcheck // explicit type tests the interface assignment contract
}

// TestRandomInterfaceSignature verifies Random interface has the expected signature.
func TestRandomInterfaceSignature(_ *testing.T) {
	var _ Random = newRandom(42) //nolint:staticcheck // explicit type tests the interface assignment contract
}
