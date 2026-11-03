package main

import "errors"

//go:generate sh -c "cd ../tools/errgen && go run . -spec ../../agent/errors.json -out ../../agent/errors_gen.go"

// ErrorClass is the category of a failure, shared by local command operations.
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
	// ClassUnknownTool: the MCP client called a tool that agent does not provide.
	ClassUnknownTool
	// ClassProcess: a helper process (wake-up, dispatcher) could not be started or failed.
	ClassProcess
)

// String returns the class name, e.g. "NotFound".
func (c ErrorClass) String() string { return "" }

// ExitCode preserves the agent CLI's generic failure status; operations with special exits map them explicitly.
func (c ErrorClass) ExitCode() int { return 0 }

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

// Sentinel returns the sentinel error of the class.
func (c ErrorClass) Sentinel() error { return nil }

// classOf returns the class and cause name of err (empty for errors that are not typed agent errors).
func classOf(_ error) (class, cause string) { return "", "" }

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
func (e *OpError[C]) Error() string { return "" }

// Unwrap exposes the class sentinel and any underlying error.
func (e *OpError[C]) Unwrap() error { return nil }

// Class returns the class of the cause.
func (e *OpError[C]) Class() ErrorClass { return 0 }

// CauseName returns the name of the cause.
func (e *OpError[C]) CauseName() string { return "" }

// asError returns e as an error value, or a nil error when e is nil (never a non-nil interface holding nil).
func (e *OpError[C]) asError() error { return nil }

// opErr builds an operation error with a formatted message.
func opErr[C opCause](_ C, _ string, _ ...any) *OpError[C] { return nil }
