package main

import (
	"math/rand"
	"sync"
	"time"
)

// Clock interface defines time boundary for deterministic testing.
type Clock interface {
	// Now returns the current time.
	Now() time.Time
}

// Random interface defines random number generation with concurrent safety.
type Random interface {
	// Int63 returns a nonnegative random int64 value.
	Int63() int64
}

// realClock implements Clock with time.Now().
type realClock struct{}

// Now returns the current time.
func (rc *realClock) Now() time.Time {
	return time.Now()
}

// newClock returns a real clock adapter.
func newClock() Clock {
	return &realClock{}
}

// mutexRandom implements Random with a mutex-protected math/rand.Source.
type mutexRandom struct {
	mu sync.Mutex
	s  rand.Source
}

// Int63 returns a nonnegative random int64 value.
func (mr *mutexRandom) Int63() int64 {
	mr.mu.Lock()
	defer mr.mu.Unlock()
	return mr.s.Int63()
}

// newRandom returns a Random source seeded with the given seed, safe for concurrent calls.
func newRandom(seed int64) Random {
	return &mutexRandom{
		s: rand.NewSource(seed),
	}
}
