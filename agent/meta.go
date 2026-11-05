package main

// MetaLineKind distinguishes decoded data from records requiring shell evaluation.
type MetaLineKind uint8

const (
	MetaBlank      MetaLineKind = iota // MetaBlank retains whitespace-only records.
	MetaComment                        // MetaComment retains comments without evaluating them.
	MetaAssignment                     // MetaAssignment contains a literal scalar assignment.
	MetaShell                          // MetaShell retains unsupported shell syntax verbatim.
)

// Valid reports whether the record classification is known.
func (k MetaLineKind) Valid() bool { return k <= MetaShell }

// MetaLine retains physical bytes as well as the decoded assignment when available.
type MetaLine struct {
	Kind    MetaLineKind // classification; shell records never execute
	Raw     string       // original bytes, including the terminating newline if present
	Key     string       // assignment name, empty for non-assignments
	Value   string       // decoded literal value, including explicitly empty values
	Encoded string       // original assignment word for consumers comparing encoded text
}

// MetaFile owns ordered records; unknown names and shell records are preserved.
type MetaFile struct {
	Lines []MetaLine // source order, including duplicate assignments
}

// parseMeta classifies literal records without executing metadata as a program.
func parseMeta(_ []byte) MetaFile { return MetaFile{} }

// Lookup returns the last literal assignment; absence differs from an empty value.
func (m MetaFile) Lookup(_ string) (string, bool) { return "", false }

// Bytes reproduces the original file, including unsupported records and final newline state.
func (m MetaFile) Bytes() []byte { return nil }

// readTaskMeta reads an existing file without inheriting any caller variables.
func readTaskMeta(_, _ string) (MetaFile, *ReadTaskMetaError) { return MetaFile{}, nil }
