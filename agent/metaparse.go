package main

import (
	"strings"
	"unicode/utf8"
)

// metaWord carries a decoded literal and its consumed source length; zero means unsupported.
type metaWord struct {
	value   string // decoded bytes, with ANSI NUL truncation applied per quoted part
	end     int    // bytes consumed, including a closing quote for quoted parts
	literal bool   // false when evaluation or incomplete syntax would be required
}

// metaExpansion distinguishes a literal dollar from Bash parameter or command syntax.
func metaExpansion(s string) bool {
	if len(s) < 2 {
		return false
	}
	return isName(s[1]) || strings.ContainsRune("{([\"'@*#?$!-", rune(s[1]))
}

// recordEnd finds an unquoted newline, ignoring quotes inside shell comments.
func recordEnd(s string) int {
	sq, ansi, dq := false, false, false
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case sq:
			if c == '\'' {
				sq = false
			}
		case ansi:
			switch c {
			case '\\':
				i++
			case '\'':
				ansi = false
			}
		case dq:
			switch c {
			case '\\':
				i++
			case '"':
				dq = false
			}
		case c == '\\':
			i++
		case c == '\'':
			sq = true
		case c == '$' && i+1 < len(s) && s[i+1] == '\'':
			ansi = true
			i++
		case c == '"':
			dq = true
		case c == '#' && metaCommentStart(s, i):
			if j := strings.IndexByte(s[i:], '\n'); j >= 0 {
				return i + j
			}
			return len(s)
		case c == '\n':
			return i
		}
	}
	return len(s)
}

// metaCommentStart checks whether an unquoted hash follows an unescaped token boundary.
func metaCommentStart(s string, i int) bool {
	if i == 0 {
		return true
	}
	if !strings.ContainsRune(" \t\n;&|()<>", rune(s[i-1])) {
		return false
	}
	escaped := false
	for j := i - 2; j >= 0 && s[j] == '\\'; j-- {
		escaped = !escaped
	}
	return !escaped
}

// isNameStart accepts the portable ASCII scalar-name prefix.
func isNameStart(c byte) bool { return c == '_' || c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' }

// isName accepts the remaining portable scalar-name bytes.
func isName(c byte) bool { return isNameStart(c) || c >= '0' && c <= '9' }

// classifyMeta retains every byte while recognizing only literal assignments.
func classifyMeta(raw string) MetaLine {
	s := strings.TrimLeft(strings.TrimSuffix(raw, "\n"), " \t")
	if s == "" {
		return MetaLine{Kind: MetaBlank, Raw: raw}
	}
	if s[0] == '#' {
		return MetaLine{Kind: MetaComment, Raw: raw}
	}
	i := 0
	if !isNameStart(s[0]) {
		return MetaLine{Kind: MetaShell, Raw: raw}
	}
	for i < len(s) && isName(s[i]) {
		i++
	}
	if i >= len(s) || s[i] != '=' {
		return MetaLine{Kind: MetaShell, Raw: raw}
	}
	key := s[:i]
	word := s[i+1:]
	decoded := decodeWord(word)
	if !decoded.literal {
		return MetaLine{Kind: MetaShell, Raw: raw, Key: key}
	}
	rest := strings.TrimLeft(word[decoded.end:], " \t")
	if rest != "" && rest[0] != '#' {
		return MetaLine{Kind: MetaShell, Raw: raw, Key: key} // KEY=a b: an environment prefix for a command
	}
	return MetaLine{Kind: MetaAssignment, Raw: raw, Key: key, Value: decoded.value, Encoded: word[:decoded.end]}
}

// decodeWord decodes one shell word made of literal parts, recording the bytes consumed.
func decodeWord(w string) metaWord {
	var b strings.Builder
	i := 0
	for i < len(w) {
		c := w[i]
		switch {
		case c == ' ' || c == '\t':
			return metaWord{value: b.String(), end: i, literal: true}
		case c == '\\':
			if i+1 >= len(w) {
				return metaWord{} // trailing backslash: continuation line
			}
			if w[i+1] == '\n' {
				i += 2 // line continuation
				continue
			}
			b.WriteByte(w[i+1])
			i += 2
		case c == '\'':
			j := strings.IndexByte(w[i+1:], '\'')
			if j < 0 {
				return metaWord{}
			}
			b.WriteString(w[i+1 : i+1+j])
			i += j + 2
		case c == '$' && i+1 < len(w) && w[i+1] == '\'':
			part := decodeMetaANSI(w[i+2:])
			if !part.literal {
				return metaWord{}
			}
			b.WriteString(part.value)
			i += 2 + part.end
		case c == '"':
			part := decodeMetaDouble(w[i+1:])
			if !part.literal {
				return metaWord{}
			}
			b.WriteString(part.value)
			i += 1 + part.end
		case c == '$' && metaExpansion(w[i:]) || c == '`' || c == ';' || c == '&' || c == '|' || c == '<' || c == '>' || c == '(' || c == ')':
			return metaWord{}
		case c == '~' && (i == 0 || w[i-1] == ':'):
			return metaWord{} // tilde expansion in an assignment
		default:
			b.WriteByte(c)
			i++
		}
	}
	return metaWord{value: b.String(), end: i, literal: true}
}

// trimNUL ends one ANSI expansion at its first NUL; later word parts still concatenate.
func trimNUL(s string) string {
	if k := strings.IndexByte(s, 0); k >= 0 {
		return s[:k]
	}
	return s
}

// decodeMetaDouble decodes "..." without expansions, consuming the closing quote.
func decodeMetaDouble(s string) metaWord {
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch c {
		case '"':
			return metaWord{value: b.String(), end: i + 1, literal: true}
		case '`':
			return metaWord{}
		case '$':
			if metaExpansion(s[i:]) {
				return metaWord{}
			}
			b.WriteByte(c)
		case '\\':
			if i+1 < len(s) && strings.IndexByte("$`\"\\\n", s[i+1]) >= 0 {
				if s[i+1] != '\n' {
					b.WriteByte(s[i+1])
				}
				i++
				continue
			}
			b.WriteByte(c)
		default:
			b.WriteByte(c)
		}
	}
	return metaWord{}
}

// decodeMetaANSI decodes the body of $'...' in a UTF-8 locale, consuming the closing quote.
func decodeMetaANSI(s string) metaWord {
	var out []byte
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c == '\'' {
			return metaWord{value: trimNUL(string(out)), end: i + 1, literal: true}
		}
		if c != '\\' || i+1 >= len(s) {
			out = append(out, c)
			continue
		}
		i++
		switch e := s[i]; e {
		case 'a':
			out = append(out, 7)
		case 'b':
			out = append(out, 8)
		case 'e', 'E':
			out = append(out, 27)
		case 'f':
			out = append(out, 12)
		case 'n':
			out = append(out, 10)
		case 'r':
			out = append(out, 13)
		case 't':
			out = append(out, 9)
		case 'v':
			out = append(out, 11)
		case '\\', '\'', '"', '?':
			out = append(out, e)
		case '0', '1', '2', '3', '4', '5', '6', '7':
			v, j := 0, i
			for j < len(s) && j < i+3 && s[j] >= '0' && s[j] <= '7' {
				v = v*8 + int(s[j]-'0')
				j++
			}
			out = append(out, byte(v))
			i = j - 1
		case 'x', 'u', 'U':
			digitLimit := map[byte]int{'x': 2, 'u': 4, 'U': 8}[e]
			v, j := 0, i+1
			for j < len(s) && j < i+1+digitLimit && isHex(s[j]) {
				v = v*16 + hexVal(s[j])
				j++
			}
			if j == i+1 {
				out = append(out, '\\', e) // no digits: kept literally
				continue
			}
			if e == 'x' {
				out = append(out, byte(v))
			} else {
				if !utf8.ValidRune(rune(v)) {
					return metaWord{} // Bash emits non-UTF-8 bytes for these values; retain the raw record.
				}
				out = utf8.AppendRune(out, rune(v))
			}
			i = j - 1
		case 'c':
			if i+1 >= len(s) || s[i+1] == '\'' {
				out = append(out, '\\', 'c')
				continue
			}
			i++
			out = append(out, s[i]&0x1f)
			if s[i] == '?' {
				out[len(out)-1] = 0x7f
			}
		default:
			out = append(out, '\\', e)
		}
	}
	return metaWord{}
}

// isHex recognizes ASCII hexadecimal digits for ANSI escapes.
func isHex(c byte) bool { return c >= '0' && c <= '9' || c >= 'a' && c <= 'f' || c >= 'A' && c <= 'F' }

// hexVal converts a previously validated hexadecimal digit.
func hexVal(c byte) int {
	switch {
	case c <= '9':
		return int(c - '0')
	case c >= 'a':
		return int(c-'a') + 10
	}
	return int(c-'A') + 10
}
