//! Loading scripts and reporting diagnostics against them.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use datascript_diagnostics::codes::{DS3001, DS3008};
use datascript_diagnostics::{
    Diagnostic, FileId, Renderer, Severity, SourceFile, SourceMap, did_you_mean,
};

use crate::EXIT_USAGE;
use crate::args::{ColorChoice, ScriptArgs};

/// A loaded script: its file in [`Sources`] and the directory its relative paths resolve
/// against (LANGUAGE.md §3.2).
#[derive(Debug, Clone)]
pub struct Script {
    pub file: FileId,
    pub base_dir: PathBuf,
}

/// Every script of one command, for rendering diagnostics against.
#[derive(Debug, Default)]
pub struct Sources {
    map: SourceMap,
}

impl Sources {
    pub fn new() -> Self {
        Sources::default()
    }

    pub fn map(&self) -> &SourceMap {
        &self.map
    }

    pub fn file(&self, script: &Script) -> &SourceFile {
        self.map
            .get(script.file)
            .expect("a script's file is in the map it was loaded into")
    }

    /// Loads the program named by `ds run` / `ds explain` arguments.
    pub fn load(&mut self, args: &ScriptArgs) -> Result<Script, Diagnostic> {
        match (&args.expr, &args.file) {
            (Some(text), _) => Ok(self.add("<expr>", text.clone(), current_dir())),
            (None, Some(path)) => self.load_path(path),
            // clap requires one of them.
            (None, None) => Ok(self.add("<expr>", String::new(), current_dir())),
        }
    }

    /// Loads a script file, or standard input for `-`.
    pub fn load_path(&mut self, path: &Path) -> Result<Script, Diagnostic> {
        if path.as_os_str() == "-" {
            let mut text = String::new();
            return match std::io::stdin().read_to_string(&mut text) {
                Ok(_) => Ok(self.add("<stdin>", text, current_dir())),
                Err(e) => Err(Diagnostic::error(DS3008, "cannot read standard input")
                    .note(format!("the system said: {e}"))),
            };
        }
        let text = std::fs::read_to_string(path).map_err(|e| unreadable(path, &e))?;
        let base_dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
        Ok(self.add(&path.display().to_string(), text, base_dir))
    }

    fn add(&mut self, name: &str, text: String, base_dir: PathBuf) -> Script {
        let file = self.map.add(name, text);
        Script { file, base_dir }
    }
}

fn current_dir() -> PathBuf {
    PathBuf::from(".")
}

/// `DS3001` for a script that does not exist (with a did-you-mean over its directory), `DS3008`
/// for one that cannot be read.
fn unreadable(path: &Path, error: &std::io::Error) -> Diagnostic {
    let shown = path.display();
    if error.kind() != std::io::ErrorKind::NotFound {
        let mut d = Diagnostic::error(DS3008, format!("cannot read `{shown}`"));
        if path.is_dir() {
            d = d.help("this is a directory; give the path of a script inside it");
        } else if error.kind() == std::io::ErrorKind::InvalidData {
            d = d.note("the file is not UTF-8 text");
        } else {
            d = d.note(format!("the system said: {error}"));
        }
        return d;
    }
    let mut d = Diagnostic::error(DS3001, format!("file `{shown}` not found"));
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let nearby: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let suggestions = did_you_mean(&file_name, nearby.iter().map(String::as_str));
    if let Some(best) = suggestions.single() {
        d = d.help(format!(
            "a file with a similar name exists: `{}`",
            path.with_file_name(best).display()
        ));
    } else if !dir.exists() {
        d = d.note(format!("the directory `{}` does not exist", dir.display()));
    }
    d
}

/// Prints diagnostics on standard error in the style `--color` asks for.
#[derive(Debug)]
pub struct Reporter {
    renderer: Renderer,
}

impl Reporter {
    pub fn new(color: ColorChoice) -> Self {
        let terminal = std::io::stderr().is_terminal();
        // https://no-color.org: set and not empty.
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let renderer = match color {
            ColorChoice::Always => Renderer::terminal(true),
            ColorChoice::Auto if terminal => Renderer::terminal(!no_color),
            ColorChoice::Never if terminal => Renderer::terminal(false),
            // Not a terminal: plain ASCII, so logs and CI output stay readable.
            ColorChoice::Auto | ColorChoice::Never => Renderer::plain(),
        };
        Reporter { renderer }
    }

    pub fn report(&self, sources: &Sources, diagnostics: &[Diagnostic]) {
        if !diagnostics.is_empty() {
            eprint!("{}", self.renderer.render_all(diagnostics, sources.map()));
        }
    }

    /// Reports a diagnostic that stops the command before it starts, and returns exit code 2.
    pub fn fail_usage(&self, sources: &Sources, diagnostic: &Diagnostic) -> u8 {
        debug_assert_eq!(diagnostic.severity, Severity::Error);
        self.report(sources, std::slice::from_ref(diagnostic));
        EXIT_USAGE
    }
}
