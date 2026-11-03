package main

import "time"

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

// newClock returns a real clock adapter.
func newClock() Clock {
	return nil
}

// newRandom returns a Random source seeded with the given seed, safe for concurrent calls.
func newRandom(seed int64) Random {
	return nil
}
