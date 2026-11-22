//! Where results go: standard output or `--output <path>`. The printing itself is
//! [`datascript_engine::printer`], shared with the language server's Run (D-054).

use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};

use datascript_diagnostics::Span;
use datascript_engine::RunStream;
use datascript_engine::printer::{self, Format};
pub use datascript_engine::printer::{Frontend, Layout, Streamed, count};

use crate::args::RunFormat;

impl From<RunFormat> for Format {
    fn from(format: RunFormat) -> Self {
        match format {
            RunFormat::Table => Format::Table,
            RunFormat::Csv => Format::Csv,
            RunFormat::Json => Format::Json,
            RunFormat::Ndjson => Format::Ndjson,
            RunFormat::Arrow => Format::Arrow,
        }
    }
}

/// Where results go: standard output or `--output <path>`.
#[derive(Debug, Clone)]
pub enum Destination {
    Stdout,
    File(PathBuf),
}

impl Destination {
    pub fn new(path: Option<&Path>) -> Self {
        match path {
            Some(path) => Destination::File(path.to_path_buf()),
            None => Destination::Stdout,
        }
    }

    /// Whether results would land on a terminal.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Destination::Stdout) && io::stdout().is_terminal()
    }

    fn open(&self) -> io::Result<Box<dyn Write + Send>> {
        match self {
            // Not `stdout().lock()`: the writer is held across awaits on a worker thread.
            Destination::Stdout => Ok(Box::new(BufWriter::new(io::stdout()))),
            Destination::File(path) => Ok(Box::new(BufWriter::new(File::create(path)?))),
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Destination::Stdout => f.write_str("standard output"),
            Destination::File(path) => write!(f, "`{}`", path.display()),
        }
    }
}

/// Writing results failed.
#[derive(Debug)]
pub enum OutputError {
    /// The reader closed the pipe (`ds run … | head`).
    BrokenPipe,
    Io {
        target: String,
        error: io::Error,
    },
}

impl OutputError {
    fn new(destination: &Destination, error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::BrokenPipe {
            OutputError::BrokenPipe
        } else {
            OutputError::Io {
                target: destination.to_string(),
                error,
            }
        }
    }
}

/// Writes `text` to standard output.
pub fn write_stdout(text: &[u8]) -> Result<(), OutputError> {
    let mut out = io::stdout().lock();
    out.write_all(text)
        .and_then(|()| out.flush())
        .map_err(|e| OutputError::new(&Destination::Stdout, e))
}

/// A [`Destination`], opened when the first result has something to write.
pub struct Sink {
    destination: Destination,
    inner: printer::Sink,
}

impl Sink {
    pub fn new(destination: Destination) -> Self {
        let opened = destination.clone();
        Sink {
            destination,
            inner: printer::Sink::lazy(move || opened.open()),
        }
    }

    /// A sink over a writer that is already open (the REPL's standard output). Errors name
    /// standard output.
    pub fn writer(out: Box<dyn Write + Send>) -> Self {
        Sink {
            destination: Destination::Stdout,
            inner: printer::Sink::writer(out),
        }
    }

    /// Flushes what was written, if anything was.
    pub fn finish(&mut self) -> Result<(), OutputError> {
        self.inner
            .finish()
            .map_err(|e| OutputError::new(&self.destination, e))
    }
}

/// [`printer::stream_results`] into `sink`, with write errors naming its destination.
pub async fn stream_results(
    sink: &mut Sink,
    layout: Layout,
    run: &mut RunStream,
    line_of: &(dyn Fn(Span) -> Option<u32> + Sync),
) -> Result<Streamed, OutputError> {
    printer::stream_results(&mut sink.inner, layout, run, line_of)
        .await
        .map_err(|e| OutputError::new(&sink.destination, e))
}
