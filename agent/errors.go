package main

import (
	"errors"
	"fmt"
)

//go:generate sh -c "cd ../tools/errgen && go run . -spec ../../agent/errors.json -out ../../agent/errors_gen.go"

// ErrorClass is the category of a failure, shared by local command operations.
type ErrorClass int

const (
	// ClassInvalidArgument: a required argument is missing or has an invalid value.
	ClassInvalidArgument ErrorClass = iota
	// ClassNotFound: the referenced object does not exist.
	ClassNotFound
	// ClassNotInTeam: a team-only operation was called by an agent that is not in a team.
	ClassNotInTeam
	// ClassPermission: the caller's role or tier does not allow the operation.
	ClassPermission
	// ClassConflict: the operation was already done or the object is taken by someone else.
	ClassConflict
	// ClassInvalidState: the object exists but its state does not allow the operation.
	ClassInvalidState
	// ClassStorage: reading or writing the state directory, a log file or stdout failed.
	ClassStorage
	// ClassUnknownTool: the requested tool does not exist.
	ClassUnknownTool
	// ClassProcess: a required external command could not be started or failed.
	ClassProcess
)

// classNames is a read-only table of diagnostic class names, indexed by ErrorClass.
var classNames = [...]string{
	"InvalidArgument", "NotFound", "NotInTeam", "Permission", "Conflict", "InvalidState", "Storage", "UnknownTool", "Process",
}

// String returns the class name, e.g. "NotFound".
func (c ErrorClass) String() string {
	if c < 0 || int(c) >= len(classNames) {
		return fmt.Sprintf("ErrorClass(%d)", int(c))
	}
	return classNames[c]
}

// ExitCode preserves the agent CLI's generic failure status; operations with special exits map them explicitly.
func (c ErrorClass) ExitCode() int { return 1 }

// Sentinel errors, one per class: every typed error of agent unwraps to the sentinel of its class.
var (
	ErrInvalidArgument = errors.New("invalid argument") // sentinel of ClassInvalidArgument
	ErrNotFound        = errors.New("not found")        // sentinel of ClassNotFound
	ErrNotInTeam       = errors.New("not in a team")    // sentinel of ClassNotInTeam
	ErrPermission      = errors.New("not allowed")      // sentinel of ClassPermission
	ErrConflict        = errors.New("conflict")         // sentinel of ClassConflict
	ErrInvalidState    = errors.New("invalid state")    // sentinel of ClassInvalidState
	ErrStorage         = errors.New("storage error")    // sentinel of ClassStorage
	ErrUnknownTool     = errors.New("unknown tool")     // sentinel of ClassUnknownTool
	ErrProcess         = errors.New("process failed")   // sentinel of ClassProcess
)

// sentinels is a read-only table mapping every class to its stable sentinel error.
var sentinels = [...]error{
	ErrInvalidArgument, ErrNotFound, ErrNotInTeam, ErrPermission, ErrConflict, ErrInvalidState, ErrStorage, ErrUnknownTool, ErrProcess,
}

// Sentinel returns the sentinel error of the class.
func (c ErrorClass) Sentinel() error {
	if c < 0 || int(c) >= len(sentinels) {
		return errors.New(c.String())
	}
	return sentinels[c]
}

// classified is implemented by every typed error of agent; logging and exit codes use it.
type classified interface {
	error
	Class() ErrorClass // the category of the failure
	CauseName() string // the precise cause (e.g. "UnknownCommand"; the class name for low-level errors)
}

// classOf returns the class and cause name of err (empty for errors that are not typed agent errors).
func classOf(err error) (class, cause string) {
	// c captures a typed failure through any standard wrapping layers.
	var c classified
	if errors.As(err, &c) {
		return c.Class().String(), c.CauseName()
	}
	return "", ""
}

// ---------------------------------------------------------------- per-operation errors

// causeInfo describes one value of a per-operation cause enum (generated in errors_gen.go).
type causeInfo struct {
	name  string     // cause name, e.g. "UnknownCommand"
	class ErrorClass // its class
}

// opCause is implemented by every per-operation cause enum generated from errors.json.
type opCause interface {
	comparable
	op() string      // the operation the causes belong to: an MCP tool or CLI subcommand name
	info() causeInfo // name and class of the value
}

// OpError is the error of one operation. C is the operation's cause enum, so a function returning
// *OpError[CommandCause] (alias CommandError) can only fail with a CommandCause.
type OpError[C opCause] struct {
	Cause   C      // why the operation failed
	Message string // what went wrong and, where possible, what to do instead (agents read it)
	Err     error  // the class sentinel, joined with an underlying error when there is one (errors.Is / errors.As)
}

// Error returns "operation: message".
func (e *OpError[C]) Error() string { return e.Cause.op() + ": " + e.Message }

// Unwrap exposes the class sentinel and any underlying error.
func (e *OpError[C]) Unwrap() error { return e.Err }

// Class returns the class of the cause.
func (e *OpError[C]) Class() ErrorClass { return e.Cause.info().class }

// CauseName returns the name of the cause.
func (e *OpError[C]) CauseName() string { return e.Cause.info().name }

// asError returns e as an error value, or a nil error when e is nil (never a non-nil interface holding nil).
func (e *OpError[C]) asError() error {
	if e == nil {
		return nil
	}
	return e
}

// opErr builds an operation error with a formatted message.
func opErr[C opCause](c C, format string, args ...any) *OpError[C] {
	return &OpError[C]{Cause: c, Message: fmt.Sprintf(format, args...), Err: c.info().class.Sentinel()}
}
