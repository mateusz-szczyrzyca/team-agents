// Errors of agentbus.
//
// Every operation — an MCP tool (quiz_start, board_claim, ...) or a CLI subcommand (team-get, inv-set, ...) —
// returns its own error type, generated from errors.json into errors_gen.go:
//
//	func (s *Server) quizStart(question string) (string, *QuizStartError)
//
// QuizStartError is OpError[QuizStartCause]; QuizStartCause is an enum of exactly the reasons quiz_start can
// fail (QuizStartNotInTeam, QuizStartNotCoordinator, QuizStartAlreadyHeld, ...). The compiler rejects any
// other cause, and the enum documents every possible failure where the function is declared.
//
// Each cause belongs to an ErrorClass (InvalidArgument, NotFound, Permission, ...). The class gives the
// sentinel error for errors.Is, the exit status of CLI subcommands and the "class" field of log records:
//
//	if e != nil && e.Cause == QuizStartNotCoordinator { ... }   // exact cause, no type assertion needed
//	errors.Is(err, ErrPermission)                                 // class, from any error value
//	var e *QuizStartError; errors.As(err, &e)                     // details from a plain error value
//
// Below the operations, helpers return concrete low-level errors: *StorageError (state and log files, stdout),
// *ValueError (an invalid enum value or timestamp) and *ProcessError (a helper process could not run). An
// operation converts them into its own cause explicitly.
//
// The plain error interface is used only where an interface requires it (slog.Handler.Handle,
// json.Unmarshaler, flag.Value) and at the MCP dispatcher; the conversion goes through asError, which keeps a
// nil pointer from becoming a non-nil error value.
//
// Plain tool *results* such as "QUEUE FULL" or "TAKEN" are not errors: they are answers the agent acts on.
package main

import (
	"errors"
	"fmt"
	"strings"
)

//go:generate sh -c "cd ../tools/errgen && go run . -spec ../../agentbus/errors.json -out ../../agentbus/errors_gen.go"

// ---------------------------------------------------------------- classes

// ErrorClass is the category of a failure, shared by all operations.
type ErrorClass int

const (
	// ClassInvalidArgument: a required argument is missing or has an invalid value.
	ClassInvalidArgument ErrorClass = iota
	// ClassNotFound: the referenced team, member, board item, investigation or worker request does not exist.
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
	// ClassUnknownTool: the MCP client called a tool that agentbus does not provide.
	ClassUnknownTool
	// ClassProcess: a helper process (wake-up, dispatcher) could not be started or failed.
	ClassProcess
	// ClassUnavailable: the hub cannot be reached after the bounded retries (multi-instance mode). The operation
	// may be retried by the caller; a lost mutation is never re-executed (see OutcomeUnknown in the design).
	ClassUnavailable
)

// classNames are the String() values of the classes, indexed by ErrorClass.
var classNames = [...]string{
	"InvalidArgument", "NotFound", "NotInTeam", "Permission", "Conflict", "InvalidState", "Storage", "UnknownTool", "Process",
	"Unavailable",
}

// String returns the class name, e.g. "NotFound".
func (c ErrorClass) String() string {
	if c < 0 || int(c) >= len(classNames) {
		return fmt.Sprintf("ErrorClass(%d)", int(c))
	}
	return classNames[c]
}

// ExitCode is the exit status of a CLI subcommand failing with this class, so that the `agent` script can
// tell "not found" (an answer) from "storage" (a real error):
//
//	2 InvalidArgument   3 NotFound   4 NotInTeam   5 Permission   6 Conflict
//	7 InvalidState      8 Storage    9 UnknownTool 10 Process      11 Unavailable
func (c ErrorClass) ExitCode() int {
	if c < 0 || int(c) >= len(classNames) {
		return 1
	}
	return int(c) + 2
}

// Sentinel errors, one per class: every typed error of agentbus unwraps to the sentinel of its class.
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
	ErrUnavailable     = errors.New("hub unavailable")  // sentinel of ClassUnavailable
)

// sentinels maps every class to its sentinel error (same order as the constants).
var sentinels = [...]error{
	ErrInvalidArgument, ErrNotFound, ErrNotInTeam, ErrPermission, ErrConflict, ErrInvalidState, ErrStorage, ErrUnknownTool, ErrProcess,
	ErrUnavailable,
}

// Sentinel returns the sentinel error of the class.
func (c ErrorClass) Sentinel() error {
	if c < 0 || int(c) >= len(sentinels) {
		return errors.New(c.String())
	}
	return sentinels[c]
}

// classified is implemented by every typed error of agentbus; logging and exit codes use it.
type classified interface {
	error
	Class() ErrorClass // the category of the failure
	CauseName() string // the precise cause (e.g. "NotCoordinator"; the class name for low-level errors)
}

// classOf returns the class and cause name of err (empty for errors that are not typed agentbus errors).
func classOf(err error) (class, cause string) {
	var c classified
	if errors.As(err, &c) {
		return c.Class().String(), c.CauseName()
	}
	return "", ""
}

// ---------------------------------------------------------------- per-operation errors

// causeInfo describes one value of a per-operation cause enum (generated in errors_gen.go).
type causeInfo struct {
	name  string     // cause name, e.g. "NotCoordinator"
	class ErrorClass // its class
}

// opCause is implemented by every per-operation cause enum generated from errors.json.
type opCause interface {
	comparable
	op() string      // the operation the causes belong to: an MCP tool or CLI subcommand name
	info() causeInfo // name and class of the value
}

// OpError is the error of one operation. C is the operation's cause enum, so a function returning
// *OpError[QuizStartCause] (alias QuizStartError) can only fail with a QuizStartCause.
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

// fromStorage converts a storage failure into the operation's storage cause (nil stays nil).
func fromStorage[C opCause](c C, err *StorageError) *OpError[C] {
	if err == nil {
		return nil
	}
	return &OpError[C]{Cause: c, Message: err.Message, Err: errors.Join(c.info().class.Sentinel(), err)}
}

// fromValue converts an invalid value into the operation's cause for it (nil stays nil).
func fromValue[C opCause](c C, err *ValueError) *OpError[C] {
	if err == nil {
		return nil
	}
	return &OpError[C]{Cause: c, Message: err.Message, Err: errors.Join(c.info().class.Sentinel(), err)}
}

// missing builds the message for required arguments that were not given (or were blank).
func missing(fields ...string) string {
	noun := "argument"
	if len(fields) > 1 {
		noun = "arguments"
	}
	return fmt.Sprintf("missing required %s: %s", noun, strings.Join(fields, ", "))
}

// toolResult converts the result of a tool handler to the dispatcher's (string, error).
func toolResult[C opCause](out string, err *OpError[C]) (string, error) {
	if err != nil {
		return "", err
	}
	return out, nil
}

// ---------------------------------------------------------------- low-level errors

// StorageError: reading or writing a file of the state directory, a log file or stdout failed.
type StorageError struct {
	Message string // what could not be done, e.g. "cannot save …/state.json"
	Err     error  // ErrStorage joined with the underlying error (os, encoding/json)
}

// Error returns the message with the underlying error.
func (e *StorageError) Error() string { return e.Message }

// Unwrap exposes ErrStorage and the underlying error.
func (e *StorageError) Unwrap() error { return e.Err }

// Class returns ClassStorage.
func (e *StorageError) Class() ErrorClass { return ClassStorage }

// CauseName returns "Storage".
func (e *StorageError) CauseName() string { return ClassStorage.String() }

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

// ValueError: a value (an enum, a timestamp, a flag) is not valid.
type ValueError struct {
	Message string // what is wrong, e.g. `unknown member state "napping"`
	Err     error  // ErrInvalidArgument, joined with the underlying error when there is one
}

// Error returns the message.
func (e *ValueError) Error() string { return e.Message }

// Unwrap exposes ErrInvalidArgument and any underlying error.
func (e *ValueError) Unwrap() error { return e.Err }

// Class returns ClassInvalidArgument.
func (e *ValueError) Class() ErrorClass { return ClassInvalidArgument }

// CauseName returns "InvalidValue".
func (e *ValueError) CauseName() string { return "InvalidValue" }

// asError returns e as an error value (a nil error when e is nil).
func (e *ValueError) asError() error {
	if e == nil {
		return nil
	}
	return e
}

// valueErr builds a ValueError.
func valueErr(format string, args ...any) *ValueError {
	return &ValueError{Message: fmt.Sprintf(format, args...), Err: ErrInvalidArgument}
}

// valueWrap builds a ValueError around an underlying error.
func valueWrap(err error, format string, args ...any) *ValueError {
	return &ValueError{Message: fmt.Sprintf(format, args...) + ": " + err.Error(), Err: errors.Join(ErrInvalidArgument, err)}
}

// ProcessError: a helper process could not be started or exited with an error.
type ProcessError struct {
	Program string // the program that was run
	Message string // what went wrong
	Err     error  // ErrProcess joined with the underlying error
}

// Error returns the message.
func (e *ProcessError) Error() string { return e.Message }

// Unwrap exposes ErrProcess and the underlying error.
func (e *ProcessError) Unwrap() error { return e.Err }

// Class returns ClassProcess.
func (e *ProcessError) Class() ErrorClass { return ClassProcess }

// CauseName returns "Process".
func (e *ProcessError) CauseName() string { return ClassProcess.String() }

// processErr builds a ProcessError around an underlying error (nil when there is none).
func processErr(program string, err error, format string, args ...any) *ProcessError {
	msg := fmt.Sprintf(format, args...)
	if err == nil {
		return &ProcessError{Program: program, Message: msg, Err: ErrProcess}
	}
	return &ProcessError{Program: program, Message: msg + ": " + err.Error(), Err: errors.Join(ErrProcess, err)}
}
