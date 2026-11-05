package main

import (
	"strings"
	"unicode"
	"unicode/utf8"
)

// literalRune reports whether Bash (glibc UTF-8 iswprint) prints r literally.
func literalRune(r rune) bool {
	return unicode.IsGraphic(r) || unicode.In(r, unicode.Cf, unicode.Co)
}

// needsANSI reports whether %q switches to $'...' (any control, DEL, invalid or non-printable rune).
func needsANSI(s string) bool {
	for i := 0; i < len(s); {
		r, n := utf8.DecodeRuneInString(s[i:])
		switch {
		case r == utf8.RuneError && n == 1:
			return true
		case r < 0x20 || r == 0x7f:
			return true
		case r >= 0x80 && !literalRune(r):
			return true
		}
		i += n
	}
	return false
}

// backslashSet are the ASCII bytes %q escapes with a backslash anywhere in the word.
const backslashSet = " !\"$&'()*,;<>?[\\]^`{|}"

// quoteMeta emits Bash printf-%q bytes in C.UTF-8; NUL cannot exist in a Bash value.
func quoteMeta(s string) (string, *ValueError) {
	if strings.IndexByte(s, 0) >= 0 {
		return "", valueErr("metadata values cannot contain NUL")
	}
	if s == "" {
		return "''", nil
	}
	if needsANSI(s) {
		return ansiQuote(s), nil
	}
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		c := s[i]
		esc := strings.IndexByte(backslashSet, c) >= 0
		if c == '#' && i == 0 {
			esc = true
		}
		if c == '~' && (i == 0 || s[i-1] == '=' || s[i-1] == ':') {
			esc = true
		}
		if esc {
			b.WriteByte('\\')
		}
		b.WriteByte(c)
	}
	return b.String(), nil
}

// ansiQuote renders the $'...' form the way Bash's ansic_quote does.
func ansiQuote(s string) string {
	var b strings.Builder
	b.WriteString("$'")
	for i := 0; i < len(s); {
		r, n := utf8.DecodeRuneInString(s[i:])
		switch {
		case r == utf8.RuneError && n == 1:
			octal(&b, s[i])
		case r < 0x80:
			c := s[i] // ASCII is one byte; avoid narrowing an arbitrary rune.
			switch c {
			case 7:
				b.WriteString(`\a`)
			case 8:
				b.WriteString(`\b`)
			case 9:
				b.WriteString(`\t`)
			case 10:
				b.WriteString(`\n`)
			case 11:
				b.WriteString(`\v`)
			case 12:
				b.WriteString(`\f`)
			case 13:
				b.WriteString(`\r`)
			case 27:
				b.WriteString(`\E`)
			case '\'':
				b.WriteString(`\'`)
			case '\\':
				b.WriteString(`\\`)
			default:
				if c < 0x20 || c == 0x7f {
					octal(&b, c)
				} else {
					b.WriteByte(c)
				}
			}
		case literalRune(r):
			b.WriteString(s[i : i+n])
		default:
			for j := i; j < i+n; j++ {
				octal(&b, s[j])
			}
		}
		i += n
	}
	b.WriteByte('\'')
	return b.String()
}

// octal writes a fixed-width byte escape, avoiding ambiguity with following digits.
func octal(b *strings.Builder, c byte) {
	b.WriteByte('\\')
	b.WriteByte('0' + c>>6)
	b.WriteByte('0' + (c>>3)&7)
	b.WriteByte('0' + c&7)
}

// formatMetaAssignment validates a scalar name and emits one physical metadata record.
func formatMetaAssignment(key, value string) (string, *ValueError) {
	if key == "" || !isNameStart(key[0]) {
		return "", valueErr("invalid metadata name %q", key)
	}
	for i := 1; i < len(key); i++ {
		if !isName(key[i]) {
			return "", valueErr("invalid metadata name %q", key)
		}
	}
	encoded, verr := quoteMeta(value)
	if verr != nil {
		return "", verr
	}
	return key + "=" + encoded + "\n", nil
}
