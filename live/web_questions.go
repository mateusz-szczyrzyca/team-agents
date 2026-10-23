// Questions files (questions-<round>.md) as the web form reads and writes them. The format and its labels are
// those of the `agent` script (validate_answers, expand_answers): "## N. title", a "**Recommended:**" part, a
// "**Verdict:**" line (older files: "**Answer:**", "**Odpowiedź:**") and "**ANSWERED**" as the last non-empty line
// once the human is done. Writing replaces only the verdicts (and the marker); every other line stays as it was.
package main

import (
	"regexp"
	"strings"
)

var (
	qHeaderRe  = regexp.MustCompile(`^## ([0-9]+)\.\s*(.*)$`)                        // "## 3. Which store?"
	qVerdictRe = regexp.MustCompile(`^\*\*\s*(Verdict|Answer|Odpowiedź)\s*:\s*\*\*`) // the verdict label
	qRecRe     = regexp.MustCompile(`^\*\*\s*Recommended\s*:\s*\*\*`)                // the recommendation label
	qLabelRe   = regexp.MustCompile(`^\*\*\s*\p{L}[\p{L} ]*:\s*\*\*`)                // any other bold label
	qMarkerRe  = regexp.MustCompile(`^\*\*\s*ANSWERED\s*\*\*\s*$`)                   // the "answers are ready" marker
	qOptionRe  = regexp.MustCompile(`^\s*(?:[-*]\s*)?\(?([a-z])[).]\s+(.+)$`)        // "a) in memory — …"
)

// qOption is one lettered option of a question ("a) …").
type qOption struct {
	Letter string // "a", "b", …
	Text   string // the option as written after the letter
}

// question is one question of a questions file.
type question struct {
	Num         string    // its number ("## N.")
	Title       string    // the text after "## N. "
	Body        string    // the lines between the title and the first label (options, context)
	Options     []qOption // the lettered options found in the body
	Recommended string    // the recommendation ("" when the question has none)
	Verdict     string    // the current verdict (a saved draft, or empty)
	verdict     int       // index of the verdict label line (-1: the question has none)
	verdictEnd  int       // index after the verdict's text
	end         int       // index after the question's last line
}

// questionsDoc is a parsed questions file.
type questionsDoc struct {
	lines     []string    // the file, line by line
	Questions []*question // the questions in file order
	Answered  bool        // the last non-empty line is the ANSWERED marker
}

// qSection is the part of a question a line belongs to while parsing.
type qSection int

const (
	qInBody    qSection = iota // between the title and the first label
	qInRec                     // the Recommended part
	qInVerdict                 // the verdict's text
	qInOther                   // another label's text
)

// parseQuestions parses the text of a questions file.
func parseQuestions(text string) *questionsDoc {
	d := &questionsDoc{lines: strings.Split(strings.TrimRight(text, "\n"), "\n")}
	var q *question
	var body, rec, ver []string
	sec := qInOther
	finish := func(i int) {
		if q == nil {
			return
		}
		if q.verdict >= 0 && sec == qInVerdict {
			q.verdictEnd = i
		}
		q.end = i
		q.Body = strings.TrimSpace(strings.Join(body, "\n"))
		q.Recommended = strings.TrimSpace(strings.Join(rec, "\n"))
		q.Verdict = strings.TrimSpace(strings.Join(ver, "\n"))
		for _, l := range body {
			if m := qOptionRe.FindStringSubmatch(l); m != nil {
				q.Options = append(q.Options, qOption{Letter: m[1], Text: strings.TrimSpace(m[2])})
			}
		}
		d.Questions = append(d.Questions, q)
	}
	for i, l := range d.lines {
		if m := qHeaderRe.FindStringSubmatch(l); m != nil {
			finish(i)
			q = &question{Num: m[1], Title: strings.TrimSpace(m[2]), verdict: -1}
			body, rec, ver, sec = nil, nil, nil, qInBody
			continue
		}
		if q == nil {
			continue // the title and the answering instructions above the first question
		}
		endVerdict := func() {
			if sec == qInVerdict {
				q.verdictEnd = i
			}
		}
		switch {
		case qMarkerRe.MatchString(l):
			endVerdict()
			sec = qInOther
		case qVerdictRe.MatchString(l):
			endVerdict()
			q.verdict, sec = i, qInVerdict
			if rest := strings.TrimSpace(qVerdictRe.ReplaceAllString(l, "")); rest != "" {
				ver = append(ver, rest)
			}
		case qRecRe.MatchString(l):
			endVerdict()
			sec = qInRec
			rec = append(rec, strings.TrimSpace(qRecRe.ReplaceAllString(l, "")))
		case qLabelRe.MatchString(l):
			endVerdict()
			sec = qInOther
		default:
			switch sec {
			case qInBody:
				body = append(body, l)
			case qInRec:
				rec = append(rec, l)
			case qInVerdict:
				ver = append(ver, l)
			case qInOther:
			}
		}
	}
	finish(len(d.lines))
	for i := len(d.lines) - 1; i >= 0; i-- {
		if strings.TrimSpace(d.lines[i]) != "" {
			d.Answered = qMarkerRe.MatchString(d.lines[i])
			break
		}
	}
	return d
}

// render returns the file with the given verdicts (by question index; questions not in the map keep theirs) and,
// when answered is set, the ANSWERED marker as the last line. Verdict labels are written as "**Verdict:**".
func (d *questionsDoc) render(verdicts map[int]string, answered bool) string {
	lines := append([]string(nil), d.lines...)
	for qi := len(d.Questions) - 1; qi >= 0; qi-- { // from the end, so earlier indexes stay valid
		v, ok := verdicts[qi]
		if !ok {
			continue
		}
		q := d.Questions[qi]
		repl := verdictLines(v)
		if q.verdict >= 0 {
			lines = append(lines[:q.verdict], append(repl, lines[q.verdictEnd:]...)...)
		} else {
			lines = append(lines[:q.end], append(append([]string{""}, repl...), lines[q.end:]...)...)
		}
	}
	out := lines[:0]
	for _, l := range lines {
		if !qMarkerRe.MatchString(l) {
			out = append(out, l)
		}
	}
	for len(out) > 0 && strings.TrimSpace(out[len(out)-1]) == "" {
		out = out[:len(out)-1]
	}
	if answered {
		out = append(out, "", "**ANSWERED**")
	}
	return strings.Join(out, "\n") + "\n"
}

// verdictLines are the lines of a verdict: the label with the first line, the rest, and a blank line.
func verdictLines(v string) []string {
	parts := strings.Split(strings.TrimSpace(v), "\n")
	out := []string{strings.TrimRight("**Verdict:** "+parts[0], " ")}
	out = append(out, parts[1:]...)
	return append(out, "")
}

// Choices of the answer form.
const (
	choiceOK  = "ok"  // accept the recommendation
	choiceOwn = "own" // the human's own answer (the text field)
)

// composeVerdict builds a verdict from the form: the choice ("ok", an option letter or "own") and the text.
func composeVerdict(q *question, choice, text string) string {
	text = strings.TrimSpace(strings.ReplaceAll(text, "\r\n", "\n"))
	join := func(first string) string {
		if text == "" {
			return first
		}
		return first + "\n" + text
	}
	switch {
	case choice == choiceOK && q.Recommended != "":
		return join("ok")
	case choice == choiceOwn || choice == "":
		return text
	}
	for _, o := range q.Options {
		if o.Letter == choice {
			return join(o.Letter + ") " + o.Text)
		}
	}
	return text
}

// splitVerdict is the reverse of composeVerdict, for pre-filling the form from a saved draft: an empty verdict
// pre-selects the recommendation when there is one.
func splitVerdict(q *question) (choice, text string) {
	v := strings.TrimSpace(q.Verdict)
	if v == "" {
		if q.Recommended != "" {
			return choiceOK, ""
		}
		return choiceOwn, ""
	}
	first, rest, _ := strings.Cut(v, "\n")
	if strings.EqualFold(strings.TrimSpace(first), "ok") && q.Recommended != "" {
		return choiceOK, strings.TrimSpace(rest)
	}
	for _, o := range q.Options {
		if strings.HasPrefix(first, o.Letter+") ") {
			return o.Letter, strings.TrimSpace(rest)
		}
	}
	return choiceOwn, v
}
