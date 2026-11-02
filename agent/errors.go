// errors.go defines the error core and error handling for agent operations.
package main

import (
	"fmt"
)

//go:generate sh -c "cd ../tools/errgen && go run . -spec ../../agent/errors.json -out ../../agent/errors_gen.go"

// ErrorClass is the category of a failure.
type ErrorClass int

const (
	// ClassInvalidArgument: a required argument is missing or has an invalid value.
	ClassInvalidArgument ErrorClass = iota
	// ClassNotFound: the referenced resource does not exist.
	ClassNotFound
	// ClassPermission: the caller does not have permission for this operation.
	ClassPermission
	// ClassStorage: reading or writing a file failed.
	ClassStorage
	// ClassProcess: a subprocess failed to start or run.
	ClassProcess
)

// classNames are the String() values of the classes.
var classNames = [...]string{
	"InvalidArgument", "NotFound", "Permission", "Storage", "Process",
}

// String returns the class name.
func (c ErrorClass) String() string {
	if c < 0 || int(c) >= len(classNames) {
		return fmt.Sprintf("ErrorClass(%d)", int(c))
	}
	return classNames[c]
}

// opCause is an interface for operation-specific cause enums.
type opCause interface {
	op() string
	info() causeInfo
	String() string
}

// causeInfo describes a single cause value.
type causeInfo struct {
	// name is the cause name, e.g. "UnknownCommand".
	name string
	// class is the ErrorClass of this cause.
	class ErrorClass
}

// OpError is a generic operation error wrapping a specific cause.
type OpError[C opCause] struct {
	// cause is the reason the operation failed.
	cause C
	// msg is the error message.
	msg string
}

// Error implements the error interface for OpError.
func (e *OpError[C]) Error() string {
	if e == nil {
		return ""
	}
	if e.msg != "" {
		return e.msg
	}
	return e.cause.String()
}

// Cause returns the cause of this operation error.
func (e *OpError[C]) Cause() C {
	if e == nil {
		var zero C
		return zero
	}
	return e.cause
}

// NewOpError creates a new operation error with the given cause and message.
func NewOpError[C opCause](cause C, msg string) *OpError[C] {
	return &OpError[C]{
		cause: cause,
		msg:   msg,
	}
}

// NewRunError creates a new RunError with the given cause and message.
func NewRunError(cause RunCause, msg string) *RunError {
	return NewOpError(cause, msg)
}
