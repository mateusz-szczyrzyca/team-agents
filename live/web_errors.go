// Errors of the web interface (agentlive web).
//
// Every operation of the web server has its own cause enum, and its functions return *OpError[thatCause] — never
// a plain error — so a signature shows what can fail and a caller (or a test) can tell the causes apart with
// errors.As. Causes of request-time operations also say which HTTP status they map to.
package main

import (
	"errors"
	"fmt"
	"net/http"
)

// webCause is implemented by the cause enums of the web operations.
type webCause interface {
	~int
	String() string // the cause name, as logged
}

// OpError is a failure of one web operation; C is that operation's cause enum.
type OpError[C webCause] struct {
	Cause   C      // why the operation failed
	Message string // what went wrong, for the log and the error page
	Err     error  // ErrWeb joined with the underlying error (if any)
}

// ErrWeb is the sentinel every OpError unwraps to.
var ErrWeb = errors.New("web interface error")

// Error returns the message.
func (e *OpError[C]) Error() string { return e.Message }

// Unwrap exposes ErrWeb and the underlying error.
func (e *OpError[C]) Unwrap() error { return e.Err }

// webErr builds an OpError; err may be nil.
func webErr[C webCause](c C, err error, format string, args ...any) *OpError[C] {
	msg := fmt.Sprintf(format, args...)
	if err != nil {
		msg += ": " + err.Error()
	}
	return &OpError[C]{Cause: c, Message: msg, Err: errors.Join(ErrWeb, err)}
}

// StartCause: why the web server could not start.
type StartCause int

const (
	StartBadListen   StartCause = iota // the listen address cannot be parsed
	StartNotLoopback                   // the listen address is not a loopback address (refused by design)
	StartListen                        // the address could not be bound
	StartAssets                        // the embedded templates or assets are broken
)

// String returns the cause name.
func (c StartCause) String() string {
	switch c {
	case StartBadListen:
		return "BadListen"
	case StartNotLoopback:
		return "NotLoopback"
	case StartListen:
		return "Listen"
	case StartAssets:
		return "Assets"
	}
	return fmt.Sprintf("StartCause(%d)", int(c))
}

// AccessCause: why a request was refused.
type AccessCause int

const (
	AccessNotLoopback      AccessCause = iota // the connection does not come from this host
	AccessUnknownUser                         // a Tailscale user that is not in --users
	AccessProxiedAnonymous                    // forwarded by a proxy without a Tailscale identity (e.g. Funnel)
	AccessCrossSite                           // a state-changing request from another site
	AccessNotHTMX                             // a state-changing request without the HX-Request header
)

// String returns the cause name.
func (c AccessCause) String() string {
	switch c {
	case AccessNotLoopback:
		return "NotLoopback"
	case AccessUnknownUser:
		return "UnknownUser"
	case AccessProxiedAnonymous:
		return "ProxiedAnonymous"
	case AccessCrossSite:
		return "CrossSite"
	case AccessNotHTMX:
		return "NotHTMX"
	}
	return fmt.Sprintf("AccessCause(%d)", int(c))
}

// HTTPStatus maps the cause to a response status.
func (c AccessCause) HTTPStatus() int { return http.StatusForbidden }

// ViewCause: why a view could not be shown.
type ViewCause int

const (
	ViewNotFound     ViewCause = iota // no such task, team, investigation or log
	ViewBadParameter                  // a malformed parameter in the URL
	ViewStorage                       // a file could not be read
)

// String returns the cause name.
func (c ViewCause) String() string {
	switch c {
	case ViewNotFound:
		return "NotFound"
	case ViewBadParameter:
		return "BadParameter"
	case ViewStorage:
		return "Storage"
	}
	return fmt.Sprintf("ViewCause(%d)", int(c))
}

// HTTPStatus maps the cause to a response status.
func (c ViewCause) HTTPStatus() int {
	switch c {
	case ViewNotFound:
		return http.StatusNotFound
	case ViewBadParameter:
		return http.StatusBadRequest
	}
	return http.StatusInternalServerError
}

// AnswerCause: why answers from the form were not saved or sent.
type AnswerCause int

const (
	AnswerNotWaiting  AnswerCause = iota // the task does not wait for answers (any more)
	AnswerNoQuestions                    // its questions file is missing or has no questions
	AnswerIncomplete                     // "send" with a question left without a verdict
	AnswerStorage                        // the questions file could not be written
	AnswerResume                         // the answers were saved, but `agent _answers_saved` failed
)

// String returns the cause name.
func (c AnswerCause) String() string {
	switch c {
	case AnswerNotWaiting:
		return "NotWaiting"
	case AnswerNoQuestions:
		return "NoQuestions"
	case AnswerIncomplete:
		return "Incomplete"
	case AnswerStorage:
		return "Storage"
	case AnswerResume:
		return "Resume"
	}
	return fmt.Sprintf("AnswerCause(%d)", int(c))
}

// HTTPStatus maps the cause to a response status.
func (c AnswerCause) HTTPStatus() int {
	switch c {
	case AnswerNotWaiting:
		return http.StatusConflict
	case AnswerNoQuestions:
		return http.StatusNotFound
	case AnswerIncomplete:
		return http.StatusUnprocessableEntity
	}
	return http.StatusInternalServerError
}
