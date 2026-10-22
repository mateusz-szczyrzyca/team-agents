package main

// Consultations: a team member puts questions to the other available normal members of its team, with a time
// limit, and collects their answers (consult_start, consult_reply, consult_results). The coordinator uses this to
// collect recommendations before it asks the human. A member that does not answer in time is shown as such
// ("NO ANSWER"), never counted as agreement.

import (
	"fmt"
	"strings"
	"time"
)

const (
	defaultConsultMinutes = 5  // consult_start's time limit when none is given
	maxConsultMinutes     = 60 // the longest time limit consult_start accepts
)

// Consult is one consultation.
type Consult struct {
	ID       string                   `json:"id"`                // "C<n>"
	From     string                   `json:"from"`              // the member who asked
	Text     string                   `json:"text"`              // what is asked
	Asked    []string                 `json:"asked"`             // the members asked: the available normal members at the start
	Started  Timestamp                `json:"started"`           // when it started
	Deadline Timestamp                `json:"deadline"`          // answers after this are marked late
	Replies  map[string]*ConsultReply `json:"replies,omitempty"` // by member name
}

// ConsultReply is one member's answer to a consultation.
type ConsultReply struct {
	Text string    `json:"text"`           // the answer
	At   Timestamp `json:"at"`             // when it was given
	Late bool      `json:"late,omitempty"` // given after the deadline
}

// consultable reports whether a member can be asked: a normal member that is working or can be woken.
func consultable(m *TeamMember) bool {
	if m.Tier != TierNormal {
		return false
	}
	switch m.State {
	case MemberActive, MemberSleeping, MemberWaking:
		return true
	}
	return false
}

// asked reports whether the member was asked in the consultation.
func (c *Consult) asked(name string) bool {
	for _, n := range c.Asked {
		if n == name {
			return true
		}
	}
	return false
}

// consultStart (MCP consult_start) sends the text to every other available normal member of the caller's team.
// minutes is the time limit (0 = defaultConsultMinutes).
func (s *Server) consultStart(text string, minutes int) (string, *ConsultStartError) {
	text = strings.TrimSpace(text)
	if text == "" {
		return "", opErr(ConsultStartMissingText, "%s", missing("text"))
	}
	if minutes < 0 || minutes > maxConsultMinutes {
		return "", opErr(ConsultStartBadMinutes, "minutes must be between 1 and %d (0 = %d)", maxConsultMinutes, defaultConsultMinutes)
	}
	if minutes == 0 {
		minutes = defaultConsultMinutes
	}
	var c *Consult
	err := transact(s.bus, ConsultStartStorage, func(st *State) *ConsultStartError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(ConsultStartNotInTeam, "you are not in a team")
		}
		var asked []string
		for _, n := range t.names() {
			if n != s.me && consultable(t.Members[n]) {
				asked = append(asked, n)
			}
		}
		if len(asked) == 0 {
			return nil // nobody to ask: not an error (the caller decides alone)
		}
		if t.Consults == nil {
			t.Consults = map[string]*Consult{}
		}
		t.NextCID++
		now := Now()
		c = &Consult{ID: fmt.Sprintf("C%d", t.NextCID), From: s.me, Text: text, Asked: asked, Started: now,
			Deadline: Timestamp{now.Add(time.Duration(minutes) * time.Minute)}, Replies: map[string]*ConsultReply{}}
		t.Consults[c.ID] = c
		for _, n := range asked {
			ev := EventConsultRequest{ID: c.ID, From: s.me, Question: text, Deadline: c.Deadline.Format("15:04")}
			if _, serr := s.bus.post(st, s.me, n, ev); serr != nil {
				return fromStorage(ConsultStartStorage, serr)
			}
		}
		return nil
	})
	if err != nil {
		return "", err
	}
	if c == nil {
		return "Nobody to consult: no other normal member of your team is available. Decide yourself.", nil
	}
	return fmt.Sprintf("Consultation %s sent to %s (answers until %s). Collect them with consult_results(id=%s, wait=true).",
		c.ID, strings.Join(c.Asked, ", "), c.Deadline.Format("15:04"), c.ID), nil
}

// consultReply (MCP consult_reply) records the caller's answer; the member who asked gets it as a message.
func (s *Server) consultReply(id, text string) (string, *ConsultReplyError) {
	id, text = strings.TrimSpace(id), strings.TrimSpace(text)
	if id == "" || text == "" {
		return "", opErr(ConsultReplyMissingArgument, "%s", missing("id", "text"))
	}
	late := false
	err := transact(s.bus, ConsultReplyStorage, func(st *State) *ConsultReplyError {
		s.touch(st)
		t := s.myTeam(st)
		if t == nil {
			return opErr(ConsultReplyNotInTeam, "you are not in a team")
		}
		c := t.Consults[id]
		if c == nil {
			return opErr(ConsultReplyNoSuchConsult, "no consultation %s in your team", id)
		}
		if !c.asked(s.me) {
			return opErr(ConsultReplyNotAsked, "you were not asked in %s", id)
		}
		if c.Replies[s.me] != nil {
			return opErr(ConsultReplyAlreadyReplied, "you already answered %s", id)
		}
		now := Now()
		late = now.After(c.Deadline.Time)
		if c.Replies == nil {
			c.Replies = map[string]*ConsultReply{} // an empty map is not stored (omitempty): it is nil after a reload
		}
		c.Replies[s.me] = &ConsultReply{Text: text, At: now, Late: late}
		_, serr := s.bus.post(st, s.me, c.From, EventConsultReply{ID: id, From: s.me, Answer: text, Late: late})
		return fromStorage(ConsultReplyStorage, serr)
	})
	if err != nil {
		return "", err
	}
	if late {
		return fmt.Sprintf("Your answer to %s was recorded as late (after the time limit).", id), nil
	}
	return fmt.Sprintf("Your answer to %s was recorded.", id), nil
}

// consultResults (MCP consult_results) shows every asked member's answer, "NO ANSWER" after the time limit; with
// wait it blocks until everyone answered or the time limit passed.
func (s *Server) consultResults(id string, wait bool) (string, *ConsultResultsError) {
	id = strings.TrimSpace(id)
	if id == "" {
		return "", opErr(ConsultResultsMissingArgument, "%s", missing("id"))
	}
	for {
		var sb strings.Builder
		complete := false
		err := transact(s.bus, ConsultResultsStorage, func(st *State) *ConsultResultsError {
			s.touch(st)
			t := s.myTeam(st)
			if t == nil {
				return opErr(ConsultResultsNotInTeam, "you are not in a team")
			}
			c := t.Consults[id]
			if c == nil {
				return opErr(ConsultResultsNoSuchConsult, "no consultation %s in your team", id)
			}
			expired := !time.Now().Before(c.Deadline.Time)
			fmt.Fprintf(&sb, "Consultation %s from %s (time limit %s):\n%s\n\n", c.ID, c.From, c.Deadline.Format("15:04"), c.Text)
			answered := 0
			for _, n := range c.Asked {
				r := c.Replies[n]
				switch {
				case r != nil && r.Late:
					answered++
					fmt.Fprintf(&sb, "- %s (%s, LATE): %s\n", n, r.At.Format("15:04"), r.Text)
				case r != nil:
					answered++
					fmt.Fprintf(&sb, "- %s (%s): %s\n", n, r.At.Format("15:04"), r.Text)
				case expired:
					fmt.Fprintf(&sb, "- %s: NO ANSWER (the time limit passed — this is not agreement)\n", n)
				default:
					fmt.Fprintf(&sb, "- %s: no answer yet\n", n)
				}
			}
			complete = answered == len(c.Asked) || expired
			return nil
		})
		if err != nil {
			return "", err
		}
		if complete || !wait {
			return sb.String(), nil
		}
		time.Sleep(2 * time.Second)
	}
}

// consultToolDefs returns the MCP definitions of the consultation tools (team members only).
func consultToolDefs(inTeam bool) []map[string]any {
	if !inTeam {
		return nil
	}
	str := func(d string) map[string]any { return map[string]any{"type": "string", "description": d} }
	obj := func(props map[string]any, req ...string) map[string]any {
		return map[string]any{"type": "object", "properties": props, "required": req}
	}
	tool := func(name, desc string, schema map[string]any) map[string]any {
		return map[string]any{"name": name, "description": desc, "inputSchema": schema}
	}
	return []map[string]any{
		tool("consult_start", "Ask the other available normal members of your team (e.g. for their recommendations before "+
			"questions go to the human). Each gets it as a message and answers with consult_reply.",
			obj(map[string]any{"text": str("What you ask — numbered questions, with the options"),
				"minutes": map[string]any{"type": "integer", "description": fmt.Sprintf("Time limit in minutes (default %d)", defaultConsultMinutes)}},
				"text")),
		tool("consult_reply", "Answer a consultation you were asked in (once).",
			obj(map[string]any{"id": str("e.g. C1"), "text": str("Your answer")}, "id", "text")),
		tool("consult_results", "The answers to a consultation; members that did not answer in time are shown as NO ANSWER. "+
			"wait=true waits until everyone answered or the time limit passed.",
			obj(map[string]any{"id": str("e.g. C1"), "wait": map[string]any{"type": "boolean", "description": "Wait for all answers / the time limit"}}, "id")),
	}
}
