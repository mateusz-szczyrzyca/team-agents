package main

import (
	"regexp"
	"strings"
	"unicode"
	"unicode/utf8"
)

// GitHubColorConfig carries the runner's raw color environment without registry IO.
type GitHubColorConfig struct {
	// NameColors is AGENT_GITHUB_NAME_COLORS: whitespace-separated name=color pairs.
	NameColors string
	// Palette is AGENT_GITHUB_COLORS: ordered whitespace-separated fallback colors.
	Palette string
}

// gitHubBashSpace matches the C.UTF-8 whitespace used by the Bash publication helpers.
const gitHubBashSpace = `[\t\n\v\f\r \x{1680}\x{2000}-\x{2006}\x{2008}-\x{200a}\x{2028}\x{2029}\x{205f}\x{3000}]`

// gitHubNameLine preserves attribution precedence when both shapes match a line.
var gitHubNameLine = regexp.MustCompile("^(" + gitHubBashSpace + "*-" + gitHubBashSpace + "+(?:agent|coordinator|member):" + gitHubBashSpace + "+|.*Review by the team-agents investigator )`([A-Za-z0-9._@-]+)`( \\(.*)$")

// gitHubRecommendationHead leaves the locale-sensitive alphabetic boundary to Go.
var gitHubRecommendationHead = regexp.MustCompile("^" + gitHubBashSpace + "*(?:#{1,6}" + gitHubBashSpace + "*)?[*_]{0,2}[Rr]ecommendation")

// RenderGitHubNames colors attribution and investigator names like gh_colour_names.
func RenderGitHubNames(body string, cfg GitHubColorConfig) string {
	if cfg.NameColors == "" {
		return body
	}
	// colors preserves last-assignment wins without treating Unicode space as shell IFS.
	colors := make(map[string]string)
	for _, entry := range strings.FieldsFunc(cfg.NameColors, shellColorSpace) {
		// name and color retain Bash's behavior for entries without an equals sign.
		name, color, found := strings.Cut(entry, "=")
		if !found {
			color = name
		}
		colors[name] = color
	}
	// first is the single line consumed by Bash read -ra, not the whole environment.
	first, _, _ := strings.Cut(cfg.Palette, "\n")
	// palette is ordered and may contain duplicates, just like the shell array.
	palette := strings.FieldsFunc(first, shellColorSpace)
	// given and holder are scoped to one publication, preventing cross-body allocation.
	given, holder := make(map[string]string), make(map[string]string)
	// lines preserves the input's exact terminal newline count.
	lines := strings.Split(body, "\n")
	for i, line := range lines {
		// match contains prefix, agent name and model suffix, or nil for ordinary prose.
		match := gitHubNameLine.FindStringSubmatch(line)
		if match == nil {
			continue
		}
		// name and color identify this attribution's remembered allocation.
		name, color := match[2], given[match[2]]
		if color == "" {
			color = colors[name]
			if color == "" && len(palette) > 0 {
				color = palette[uint64(gitHubNameChecksum(name))%uint64(len(palette))]
			}
			if color != "" && holder[color] != "" {
				for _, candidate := range palette {
					if holder[candidate] == "" {
						color = candidate
						break
					}
				}
			}
			if color != "" {
				given[name], holder[color] = color, name
			}
		}
		if color != "" {
			lines[i] = match[1] + `$\color{` + color + `}{\textbf{` + strings.ReplaceAll(name, "_", `\_`) + `}}$` + match[3]
		}
	}
	return strings.Join(lines, "\n")
}

// shellColorSpace restricts raw environment splitting to the default shell IFS.
func shellColorSpace(r rune) bool { return r == ' ' || r == '\t' || r == '\n' }

// isGitHubRecommendationHead implements the Bash locale's alphabetic word boundary.
func isGitHubRecommendationHead(line string) bool {
	// end is the byte offset after the matching ASCII heading, if any.
	end := gitHubRecommendationHead.FindStringIndex(line)
	if end == nil {
		return false
	}
	if end[1] == len(line) {
		return true
	}
	// next must include derived alphabetic marks and letter numbers, as libc does.
	next, _ := utf8.DecodeRuneInString(line[end[1]:])
	return !unicode.IsLetter(next) && !unicode.In(next, unicode.Nl, unicode.Other_Alphabetic)
}

// RenderGitHubRecommendation colors the first eligible verdict in a review.
func RenderGitHubRecommendation(body string) string {
	// seeking becomes false once a heading has been encountered.
	seeking := true
	// lines preserves empty and terminal lines while allowing one replacement.
	lines := strings.Split(body, "\n")
	for i, line := range lines {
		// heading is also accepted repeatedly before the first verdict-bearing line.
		heading := isGitHubRecommendationHead(line)
		if seeking && !heading {
			continue
		}
		seeking = false
		if strings.TrimFunc(line, isBashSpace) == "" {
			continue
		}
		if strings.Contains(line, `\color{`) {
			break
		}
		// lower supports case-insensitive verdicts without changing the original output.
		lower := strings.ToLower(line)
		// best and word retain the earliest eligible phrase in the line.
		best, word := -1, ""
		for _, phrase := range []string{"ready to merge", "needs changes", "needs fixes"} {
			// offset is in the lowercase copy; convert it to runes before slicing the original.
			offset := strings.Index(lower, phrase)
			if offset >= 0 && (best < 0 || offset < best) {
				best, word = offset, phrase
			}
		}
		if best < 0 {
			if !heading {
				break
			}
			continue
		}
		// runes retains Bash's character-based slicing even when lowercase changes byte width.
		runes := []rune(line)
		// start locates the ASCII verdict in the original character sequence.
		start := utf8.RuneCountInString(lower[:best])
		// pre and post keep all non-verdict text byte-for-byte.
		pre, post := string(runes[:start]), string(runes[start+len(word):])
		// bold removes the matching first closer, even when it is later in the suffix.
		bold := ""
		if strings.HasSuffix(pre, "**") || strings.HasSuffix(pre, "__") {
			bold = pre[len(pre)-2:]
			pre = pre[:len(pre)-2]
		}
		pre = strings.TrimRightFunc(pre, isBashSpace)
		for _, icon := range []string{"✅", "🔧", "🧩"} {
			pre = strings.TrimSuffix(pre, icon)
		}
		pre = strings.TrimRightFunc(pre, isBashSpace)
		if bold != "" {
			post = strings.Replace(post, bold, "", 1)
		}
		// icon and color use the existing public verdict vocabulary.
		icon, color := "✅", `$\color{green}{\textbf{Ready to merge}}$`
		if word != "ready to merge" {
			icon, color = "🔧", `$\color{red}{\textbf{Needs changes}}$`
			if strings.Contains(lower, "incomplete") {
				icon = "🧩"
			}
		}
		if pre != "" {
			pre += " "
		}
		lines[i] = pre + icon + " " + color + post
		break
	}
	return strings.Join(lines, "\n")
}

// TransformPublishBody removes generated attribution before coloring agent names.
func TransformPublishBody(body string, cfg GitHubColorConfig) string {
	return RenderGitHubNames(RemoveGeneratedLines(body), cfg)
}

// TransformReviewBody inserts the byline, colors the verdict, then publishes the body.
func TransformReviewBody(body, byline string, cfg GitHubColorConfig) string {
	return TransformPublishBody(RenderGitHubRecommendation(InsertReviewByline(body, byline)), cfg)
}

// gitHubNameChecksum computes the POSIX cksum value used for palette fallback.
func gitHubNameChecksum(name string) uint32 {
	// crc starts unreflected and folds the byte length before the final complement.
	var crc uint32
	// feed applies the POSIX polynomial in most-significant-bit order.
	feed := func(b byte) {
		crc ^= uint32(b) << 24
		for range 8 {
			if crc&0x80000000 != 0 {
				crc = crc<<1 ^ 0x04c11db7
			} else {
				crc <<= 1
			}
		}
	}
	for i := range len(name) {
		feed(name[i])
	}
	for length := len(name); length != 0; length >>= 8 {
		feed(byte(length))
	}
	return ^crc
}
