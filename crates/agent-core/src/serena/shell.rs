//! The files a line of the shell reads: its commands told apart, and of each its options, its pattern and its
//! redirections taken off, so that only the files and directories it names are left.

use super::{has_text_extension, is_code_directory, is_code_file, is_text_extension};

/// A command of the shell that searches or reads files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reader {
    /// `grep`: its first operand is a pattern, unless `-e` or `-f` gave one.
    Grep,
    /// `rg`, with the same operands as `grep`.
    Rg,
    /// `sed`: it reads a file only with `-n` (and not with `-i`); its first operand is a script, unless `-e` or `-f`
    /// gave one.
    Sed,
    /// `cat`: every operand is a file.
    Cat,
    /// `head`: every operand is a file, and `-n` and `-c` take a value.
    Head,
    /// `tail`: like `head`.
    Tail,
    /// `find`: it names no file of its own; the files it is asked for by name are the ones it reads.
    Find,
}

impl Reader {
    /// The reader that `name`, the last part of a command's path, is.
    fn named(name: &str) -> Option<Self> {
        match name {
            "grep" => Some(Self::Grep),
            "rg" => Some(Self::Rg),
            "sed" => Some(Self::Sed),
            "cat" => Some(Self::Cat),
            "head" => Some(Self::Head),
            "tail" => Some(Self::Tail),
            "find" => Some(Self::Find),
            _ => None,
        }
    }

    /// The letters of the short options that take a value: the value is the rest of the word, or the next one.
    const fn short_values(self) -> &'static str {
        match self {
            Self::Grep => "efABCmdD",
            Self::Rg => "efgtTABCmjMdE",
            Self::Sed => "efl",
            Self::Head | Self::Tail => "nc",
            Self::Cat | Self::Find => "",
        }
    }

    /// The long options that take a value: `--name=value`, or `--name value`.
    const fn long_values(self) -> &'static [&'static str] {
        match self {
            Self::Grep => &[
                "regexp",
                "file",
                "after-context",
                "before-context",
                "context",
                "max-count",
                "include",
                "exclude",
                "exclude-dir",
                "directories",
                "devices",
            ],
            Self::Rg => &[
                "regexp",
                "file",
                "glob",
                "iglob",
                "type",
                "type-not",
                "after-context",
                "before-context",
                "context",
                "max-count",
                "threads",
                "max-depth",
                "max-filesize",
                "encoding",
            ],
            Self::Sed => &["expression", "file", "line-length"],
            Self::Head | Self::Tail => &["lines", "bytes"],
            Self::Cat | Self::Find => &[],
        }
    }

    /// Whether the first operand is a pattern or a script, not a file.
    const fn has_pattern(self) -> bool {
        match self {
            Self::Grep | Self::Rg | Self::Sed => true,
            Self::Cat | Self::Head | Self::Tail | Self::Find => false,
        }
    }
}

/// What an option of a reader means for the files it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meaning {
    /// It gives the pattern or the script, so that no operand is one.
    Pattern,
    /// It filters the files by a glob (`--glob`, `--include`).
    Glob,
    /// It filters the files by the name of a type (`--type`).
    Type,
    /// `sed -n`: only the lines asked for are printed.
    Quiet,
    /// `sed -i`: the file is changed, not read.
    InPlace,
    /// Anything else: it changes nothing for the files.
    Other,
}

impl Meaning {
    /// The meaning of the short option `letter` of `reader`.
    fn of_short(reader: Reader, letter: char) -> Self {
        match (reader, letter) {
            (Reader::Grep | Reader::Rg | Reader::Sed, 'e' | 'f') => Self::Pattern,
            (Reader::Rg, 'g') => Self::Glob,
            (Reader::Rg, 't') => Self::Type,
            (Reader::Sed, 'n') => Self::Quiet,
            (Reader::Sed, 'i') => Self::InPlace,
            _ => Self::Other,
        }
    }

    /// The meaning of the long option `--key` of `reader`.
    fn of_long(reader: Reader, key: &str) -> Self {
        match (reader, key) {
            (Reader::Grep | Reader::Rg | Reader::Sed, "regexp" | "expression" | "file") => Self::Pattern,
            (Reader::Grep | Reader::Rg, "include") | (Reader::Rg, "glob" | "iglob") => Self::Glob,
            (Reader::Rg, "type") => Self::Type,
            (Reader::Sed, "quiet" | "silent") => Self::Quiet,
            (Reader::Sed, key) if key.starts_with("in-place") => Self::InPlace,
            _ => Self::Other,
        }
    }
}

/// The arguments of one command, as its options left them.
#[derive(Debug, Default)]
struct Arguments {
    /// The words that are no option, in order: the pattern first for a reader that has one.
    operands: Vec<String>,
    /// The globs of code that filter the files (`-g '*.rs'`): files of code the search names.
    code_globs: Vec<String>,
    /// Whether a filter limits the search to text (`--glob '*.md'`, `--type md`).
    text_only: bool,
    /// Whether `-e` or `-f` gave the pattern, so that the first operand is a file.
    pattern_given: bool,
    /// What `sed`'s options say it does with the file; `Printing::Everything` for any other command.
    printing: Printing,
}

/// What a `sed` prints, and whether it changes its file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Printing {
    /// No `-n` and no `-i`: it prints the whole file as edited.
    #[default]
    Everything,
    /// `-n`: only the lines asked for, which is a read.
    Quiet,
    /// `-i`: it changes the file, which is no read, whatever else is given.
    InPlace,
}

impl Arguments {
    /// Takes the option of meaning `meaning` with its `value`, if it has one.
    fn note(&mut self, meaning: Meaning, value: Option<&str>) {
        match (meaning, value) {
            (Meaning::Pattern, _) => self.pattern_given = true,
            (Meaning::Glob, Some(glob)) => self.filter_by_glob(glob),
            (Meaning::Type, Some(kind)) => self.text_only |= is_text_extension(kind),
            (Meaning::Quiet, _) if self.printing == Printing::Everything => self.printing = Printing::Quiet,
            (Meaning::InPlace, _) => self.printing = Printing::InPlace,
            (Meaning::Quiet | Meaning::Glob | Meaning::Type | Meaning::Other, _) => {}
        }
    }

    /// Takes the glob `glob` of a filter: of text, it limits the search to text; of code, it names files of code; an
    /// exclusion (`!*.md`) says nothing about what is searched.
    fn filter_by_glob(&mut self, glob: &str) {
        if glob.starts_with('!') {
            return;
        }
        if has_text_extension(glob) {
            self.text_only = true;
        } else if is_code_file(glob) {
            self.code_globs.push(glob.to_owned());
        }
    }

    /// Reads `args`, the words after the command's name, for `reader`.
    fn read(reader: Reader, args: &[String]) -> Self {
        let mut parsed = Self::default();
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            if arg == "--" {
                parsed.operands.extend(rest.by_ref().cloned());
                break;
            }
            if let Some(long) = arg.strip_prefix("--") {
                let (key, attached) = long
                    .split_once('=')
                    .map_or((long, None), |(key, value)| (key, Some(value)));
                let value = if reader.long_values().contains(&key) {
                    attached.map(str::to_owned).or_else(|| rest.next().cloned())
                } else {
                    None
                };
                parsed.note(Meaning::of_long(reader, key), value.as_deref());
            } else if let Some(cluster) = arg.strip_prefix('-').filter(|cluster| !cluster.is_empty()) {
                parsed.read_cluster(reader, cluster, &mut rest);
            } else {
                parsed.operands.push(arg.clone());
            }
        }
        parsed
    }

    /// Reads the short options of one word (`-rn`, `-A3`, `-e`): up to the first that takes a value, which is the rest of
    /// the word or the next of `rest`.
    fn read_cluster<'a>(&mut self, reader: Reader, cluster: &str, rest: &mut impl Iterator<Item = &'a String>) {
        let mut letters = cluster.chars();
        while let Some(letter) = letters.next() {
            let meaning = Meaning::of_short(reader, letter);
            if !reader.short_values().contains(letter) {
                self.note(meaning, None);
                continue;
            }
            let attached = letters.as_str();
            let value = if attached.is_empty() {
                rest.next().cloned()
            } else {
                Some(attached.to_owned())
            };
            self.note(meaning, value.as_deref());
            return;
        }
    }
}

/// The files and directories of code the command `words` (its name first) reads, as it names them.
fn segment_names(words: &[String]) -> Vec<String> {
    let Some((first, args)) = words.split_first() else {
        return Vec::new();
    };
    let name = first.rsplit_once('/').map_or(first.as_str(), |(_, last)| last);
    let Some(reader) = Reader::named(name) else {
        return Vec::new();
    };
    if reader == Reader::Find {
        return args.iter().filter(|arg| is_code_file(arg)).cloned().collect();
    }
    let parsed = Arguments::read(reader, args);
    let reads = reader != Reader::Sed || parsed.printing == Printing::Quiet;
    if parsed.text_only || !reads {
        return Vec::new();
    }
    let pattern = usize::from(reader.has_pattern() && !parsed.pattern_given);
    parsed
        .code_globs
        .into_iter()
        .chain(
            parsed
                .operands
                .into_iter()
                .skip(pattern)
                .filter(|operand| is_code_file(operand) || is_code_directory(operand)),
        )
        .collect()
}

/// The files and directories of code that the commands of `line` read, as it names them: every simple command of the line
/// (they are told apart by `|`, `||`, `&&`, `;`, `&` and a new line) on its own, so that a search that writes into a pipe
/// counts what it searched and the words of a command that reads nothing count nothing.
pub(super) fn command_filenames(line: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut words = Vec::new();
    for token in tokens(line) {
        match token {
            Token::Word(word) => words.push(word),
            Token::Break => {
                names.extend(segment_names(&std::mem::take(&mut words)));
            }
        }
    }
    names.extend(segment_names(&words));
    names
}

/// One piece of a line of the shell.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    /// A word, its quotes and escapes taken off.
    Word(String),
    /// The end of a simple command: `|`, `||`, `&&`, `;`, `&`, a new line, a parenthesis.
    Break,
}

/// The tokens of a line: words, with what is inside quotes one word, and no redirection (`>`, `2>&1`, `< file`) nor its
/// target.
fn tokens(line: &str) -> Vec<Token> {
    let mut lexer = Lexer::default();
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), close) if open == close => quote = None,
            // A backslash at the end of the line escapes nothing and stays a backslash.
            (Some('"') | None, '\\') => lexer.push(chars.next().unwrap_or('\\')),
            (Some(_), inside) => lexer.push(inside),
            (None, '\'' | '"') => {
                quote = Some(c);
                lexer.started = true;
            }
            (None, '>' | '<') => {
                lexer.redirect();
                while chars.next_if(|next| matches!(next, '<' | '>' | '&' | '|')).is_some() {}
            }
            (None, '|' | ';' | '&' | '(' | ')' | '\n') => lexer.break_command(),
            (None, blank) if blank.is_whitespace() => lexer.end_word(),
            (None, other) => lexer.push(other),
        }
    }
    lexer.end_word();
    lexer.tokens
}

/// The state of [`tokens`].
#[derive(Debug, Default)]
struct Lexer {
    /// The tokens so far.
    tokens: Vec<Token>,
    /// The word being read.
    word: String,
    /// Whether a word is being read: an empty pair of quotes is a word too.
    started: bool,
    /// Whether the next word is the target of a redirection and is dropped.
    skip_next: bool,
}

impl Lexer {
    /// Adds `c` to the word being read.
    fn push(&mut self, c: char) {
        self.word.push(c);
        self.started = true;
    }

    /// Ends the word being read, if any; the target of a redirection is dropped.
    fn end_word(&mut self) {
        if self.started {
            let word = std::mem::take(&mut self.word);
            if !std::mem::take(&mut self.skip_next) {
                self.tokens.push(Token::Word(word));
            }
        }
        self.started = false;
    }

    /// Ends the simple command: the word being read, and a redirection that has no target.
    fn break_command(&mut self) {
        self.end_word();
        self.skip_next = false;
        self.tokens.push(Token::Break);
    }

    /// Meets a redirection operator: a file descriptor before it (`2>`) is no word, and its target is dropped.
    fn redirect(&mut self) {
        if self.started && self.word.chars().all(|c| c.is_ascii_digit()) {
            self.word.clear();
            self.started = false;
        } else {
            self.end_word();
        }
        self.skip_next = true;
    }
}
