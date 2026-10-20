// Errors of agentlive.
//
// The panel does not run operations on the shared state; it only reads files every second. Its failures are
// therefore of two concrete kinds, and functions return or log exactly these types instead of a plain error:
//
//   - *StorageError: a file (meta.env, activity.jsonl, state.json, a notebook, a log destination) could not
//     be read or written;
//   - *ValueError: a value read from a file is malformed (a timestamp, a counter, a JSON line).
//
// Both unwrap to a sentinel (ErrStorage, ErrInvalidValue) for errors.Is. The plain error interface appears
// only where an interface requires it (slog.Handler.Handle) and in callbacks of Bubble Tea.
package main

import (
	"errors"
	"fmt"
)

// ErrorClass is the category of a failure.
type ErrorClass int

const (
	ClassStorage      ErrorClass = iota // a file could not be read or written
	ClassInvalidValue                   // a value read from a file is malformed
)

// String returns the class name.
func (c ErrorClass) String() string {
	switch c {
	case ClassStorage:
		return "Storage"
	case ClassInvalidValue:
		return "InvalidValue"
	}
	return fmt.Sprintf("ErrorClass(%d)", int(c))
}

// Sentinel errors, one per class.
var (
	ErrStorage      = errors.New("storage error") // sentinel of ClassStorage
	ErrInvalidValue = errors.New("invalid value") // sentinel of ClassInvalidValue
)

// classified is implemented by every typed error of agentlive; logging uses it.
type classified interface {
	error
	Class() ErrorClass // the category of the failure
}

// StorageError: a file could not be read or written.
type StorageError struct {
	Message string // what could not be done, with the underlying error
	Err     error  // ErrStorage joined with the underlying error
}

// Error returns the message.
func (e *StorageError) Error() string { return e.Message }

// Unwrap exposes ErrStorage and the underlying error.
func (e *StorageError) Unwrap() error { return e.Err }

// Class returns ClassStorage.
func (e *StorageError) Class() ErrorClass { return ClassStorage }

// asError returns e as an error value (a nil error when e is nil).
func (e *StorageError) asError() error {
	if e == nil {
		return nil
	}
	return e
}

// storageErr builds a StorageError around an underlying error.
func storageErr(err error, format string, args ...any) *StorageError {
	return &StorageError{Message: fmt.Sprintf(format, args...) + ": " + err.Error(), Err: errors.Join(ErrStorage, err)}
}

// ValueError: a value read from a file is malformed.
type ValueError struct {
	Message string // what is wrong, with the underlying error
	Err     error  // ErrInvalidValue joined with the underlying error
}

// Error returns the message.
func (e *ValueError) Error() string { return e.Message }

// Unwrap exposes ErrInvalidValue and the underlying error.
func (e *ValueError) Unwrap() error { return e.Err }

// Class returns ClassInvalidValue.
func (e *ValueError) Class() ErrorClass { return ClassInvalidValue }

// valueWrap builds a ValueError around an underlying error.
func valueWrap(err error, format string, args ...any) *ValueError {
	return &ValueError{Message: fmt.Sprintf(format, args...) + ": " + err.Error(), Err: errors.Join(ErrInvalidValue, err)}
}
