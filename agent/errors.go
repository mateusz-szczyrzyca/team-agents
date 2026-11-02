// errors.go defines the error core and error handling for agent operations.
package main

// RunError is an operation error for the Run operation. It wraps a RunCause.
type RunError struct {
	// cause is the reason for the error.
	cause RunCause
	// msg is the error message.
	msg string
}

// RunCause is an enum representing the possible failure causes of the Run operation.
type RunCause int

const (
	// RunUnknownCommand means the command was not recognized.
	RunUnknownCommand RunCause = iota
	// RunStorage means a file system or I/O operation failed.
	RunStorage
)

// Error implements the error interface for RunError.
func (e *RunError) Error() string {
	if e == nil {
		return ""
	}
	return e.msg
}

// Cause returns the RunCause of this error.
func (e *RunError) Cause() RunCause {
	if e == nil {
		return RunStorage
	}
	return e.cause
}

// NewRunError creates a new RunError with the given cause and message.
func NewRunError(cause RunCause, msg string) *RunError {
	return &RunError{
		cause: cause,
		msg:   msg,
	}
}
