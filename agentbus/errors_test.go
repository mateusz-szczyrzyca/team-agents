package main

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestErrorClassStringsAndExitCodes(t *testing.T) {
	want := map[ErrorClass]struct {
		name string
		code int
	}{
		ClassInvalidArgument: {"InvalidArgument", 2}, ClassNotFound: {"NotFound", 3}, ClassNotInTeam: {"NotInTeam", 4},
		ClassPermission: {"Permission", 5}, ClassConflict: {"Conflict", 6}, ClassInvalidState: {"InvalidState", 7},
		ClassStorage: {"Storage", 8}, ClassUnknownTool: {"UnknownTool", 9}, ClassProcess: {"Process", 10},
		ErrorClass(42): {"ErrorClass(42)", 1},
	}
	for c, w := range want {
		if c.String() != w.name || c.ExitCode() != w.code {
			t.Errorf("%d: %q/%d, want %q/%d", int(c), c.String(), c.ExitCode(), w.name, w.code)
		}
	}
}

func TestEveryClassHasItsOwnSentinel(t *testing.T) {
	if len(sentinels) != len(classNames) {
		t.Fatalf("%d sentinels for %d classes", len(sentinels), len(classNames))
	}
	for i := range sentinels {
		for j := range sentinels {
			if got := errors.Is(ErrorClass(i).Sentinel(), sentinels[j]); got != (i == j) {
				t.Errorf("class %v vs sentinel %d: %v", ErrorClass(i), j, got)
			}
		}
	}
}

// TestGeneratedCauses checks the generated enums of a few operations: names, classes and the Storage cause
// that every state-touching operation gets.
func TestGeneratedCauses(t *testing.T) {
	if QuizStartNotCoordinator.String() != "NotCoordinator" || QuizStartNotCoordinator.info().class != ClassPermission ||
		QuizStartNotCoordinator.op() != "quiz_start" {
		t.Fatalf("QuizStartNotCoordinator: %+v %s", QuizStartNotCoordinator.info(), QuizStartNotCoordinator.op())
	}
	if BoardClaimAlreadyTaken.info().class != ClassConflict || CmdTeamGetNoSuchTeam.info().class != ClassNotFound {
		t.Fatal("wrong classes in the generated tables")
	}
	if QuizStartStorage.info().class != ClassStorage || CmdTeamGetStorage.info().class != ClassStorage {
		t.Fatal("the Storage cause must be generated for every state-touching operation")
	}
	if ToolCallUnknownTool.op() != "tools/call" || CliUnknownSubcommand.op() != "agentbus" {
		t.Fatal("wrong operation names")
	}
}

func TestOpErrorFormattingAndMatching(t *testing.T) {
	e := opErr(BoardClaimNoSuchItem, "no board item %s", "B7")
	if e.Error() != "board_claim: no board item B7" || e.Class() != ClassNotFound || e.CauseName() != "NoSuchItem" {
		t.Fatalf("%q %v %q", e.Error(), e.Class(), e.CauseName())
	}
	if !errors.Is(e, ErrNotFound) || errors.Is(e, ErrPermission) {
		t.Fatal("wrong sentinel matching")
	}
	var plain error = e
	var back *BoardClaimError
	if !errors.As(plain, &back) || back.Cause != BoardClaimNoSuchItem {
		t.Fatal("errors.As must return the typed error")
	}
	class, cause := classOf(errors.Join(errors.New("context"), plain))
	if class != "NotFound" || cause != "NoSuchItem" {
		t.Fatalf("classOf through errors.Join: %q %q", class, cause)
	}
}

// TestNilTypedErrorsStayNil guards against Go's typed-nil trap: a nil *OpError must become a nil error.
func TestNilTypedErrorsStayNil(t *testing.T) {
	var e *QuizStartError
	if e.asError() != nil {
		t.Fatal("asError of a nil *OpError must be a nil error")
	}
	if out, err := toolResult("fine", e); err != nil || out != "fine" {
		t.Fatalf("toolResult(nil) = %q, %v", out, err)
	}
	var s *StorageError
	var v *ValueError
	if s.asError() != nil || v.asError() != nil {
		t.Fatal("asError of nil low-level errors must be nil")
	}
	if fromStorage(QuizStartStorage, nil) != nil || fromValue(BoardAddInvalidLevel, nil) != nil {
		t.Fatal("converting a nil error must give nil")
	}
}

func TestLowLevelErrorsKeepTheUnderlyingError(t *testing.T) {
	serr := storageErr(fs.ErrPermission, "cannot save %s", "state.json")
	if !errors.Is(serr, ErrStorage) || !errors.Is(serr, fs.ErrPermission) || !strings.HasPrefix(serr.Error(), "cannot save state.json: ") {
		t.Fatalf("StorageError: %v", serr)
	}
	op := fromStorage(CheckpointStorage, serr)
	if op.Cause != CheckpointStorage || !errors.Is(op, fs.ErrPermission) || !errors.Is(op, ErrStorage) {
		t.Fatalf("the operation error must keep the underlying error: %v", op)
	}
	var back *StorageError
	if !errors.As(op, &back) || back != serr {
		t.Fatal("errors.As must find the StorageError inside the operation error")
	}
	verr := valueErr("unknown tier %q", "boss")
	if !errors.Is(verr, ErrInvalidArgument) || verr.Class() != ClassInvalidArgument {
		t.Fatalf("ValueError: %v", verr)
	}
	if fromValue(BoardAddInvalidLevel, verr).Cause != BoardAddInvalidLevel {
		t.Fatal("fromValue must use the given cause")
	}
}

func TestMissingMessage(t *testing.T) {
	if got := missing("title"); got != "missing required argument: title" {
		t.Fatalf("got %q", got)
	}
	if got := missing("to", "text"); got != "missing required arguments: to, text" {
		t.Fatalf("got %q", got)
	}
}

// TestQuizStartCauses exercises every cause of one operation, as an example of testing exact causes.
func TestQuizStartCauses(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead, m := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T"))

	failsWith(t, QuizStartMissingQuestion)(lead.quizStart("  "))
	failsWith(t, QuizStartNotInTeam)(server(b, "solo").quizStart("q"))
	failsWith(t, QuizStartNotCoordinator)(lead.quizStart("q")) // nobody has claimed the lead yet
	chk(t).ok(toolResult(lead.claimLead()))
	failsWith(t, QuizStartNotCoordinator)(m.quizStart("q"))
	chk(t).ok(toolResult(lead.quizStart("q")))
	failsWith(t, QuizStartAlreadyHeld)(lead.quizStart("again"))

	mustWith(t, b, func(st *State) { st.Teams["T"].Quiz, st.Teams["T"].Resumed = nil, "T0" })
	failsWith(t, QuizStartResumedTeam)(lead.quizStart("q"))

	// a state file that cannot be decoded is the operation's storage cause, with the decoding error inside
	if err := os.WriteFile(filepath.Join(b.dir, "state.json"), []byte("{broken"), 0o600); err != nil {
		t.Fatal(err)
	}
	e := failure(t, QuizStartStorage)(lead.quizStart("q"))
	var syntax *json.SyntaxError
	if !errors.As(e, &syntax) {
		t.Fatalf("the JSON syntax error must stay reachable: %v", e)
	}
}

func TestDispatcherErrors(t *testing.T) {
	s := server(newBus(t), "solo")
	_, err := s.call("board_claim", map[string]any{"id": "B1"})
	if e := toolFailure(t, err, BoardClaimNotInTeam); !strings.HasPrefix(e.Error(), "board_claim: ") {
		t.Fatalf("Error() = %q", e.Error())
	}
	_, err = s.call("no_such_tool", nil)
	expectToolError(t, err, ToolCallUnknownTool)
	_, err = s.call("board_add", map[string]any{"details": "x"})
	if e := toolFailure(t, err, BoardAddMissingTitle); e.Message != "missing required argument: title" {
		t.Fatalf("message = %q", e.Message)
	}
}

func TestErrorCausesOfOperations(t *testing.T) {
	b := newBus(t)
	captureWakes(t)
	createTeam(t, b, "T", map[string][3]string{"lead": {"normal", "a", "active"}, "m": {"normal", "b", "active"}})
	lead, m := server(b, "lead", inTeam("T")), server(b, "m", inTeam("T"))
	solo := server(b, "solo")

	failsWith(t, BoardListNotInTeam)(solo.boardList())
	failsWith(t, TeamStatusNotInTeam)(solo.teamStatus())
	failsWith(t, QuizAnswerNoQuiz)(m.quizAnswer("42"))
	chk(t).ok(toolResult(lead.claimLead()))
	failsWith(t, TeamExcludeNotCoordinator)(m.teamExclude("lead", "x"))
	failsWith(t, TeamExcludeNotMember)(lead.teamExclude("ghost", "x"))
	failsWith(t, BoardClaimNoSuchItem)(m.boardClaim("B9"))
	failsWith(t, BoardAddMissingTitle)(lead.boardAdd("", "details", "easy"))
	failsWith(t, BoardAddInvalidLevel)(lead.boardAdd("t", "details", "hard"))
	failsWith(t, RecordDecisionNoSuchInvestigation)(solo.recordDecision(42, "d", "j"))
	failsWith(t, RecordDecisionMissingArgument)(solo.recordDecision(42, "", ""))
	inv := server(b, "inv", func(s *Server) { s.role = RoleInvestigator })
	failsWith(t, RequestInvestigationInvestigator)(inv.requestInvestigation("q", "", false))
	failsWith(t, RecordDecisionInvestigator)(inv.recordDecision(1, "d", "j"))
	failsWith(t, SendMessageNotInTeam)(solo.send("team", "hi"))
	failsWith(t, NotebookAppendMissingText)(solo.notebookAppend(" ", ""))
}
