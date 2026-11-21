//! The panic boundary (ARCHITECTURE.md § Engine abstraction): a bug in the compiler or an engine
//! becomes a `DS9001` diagnostic instead of taking down the LSP or `ds ui`.
//!
//! On native targets this is `catch_unwind`. On wasm32 panics abort, so nothing is caught here;
//! the WASM runtime (M5-01) installs a panic hook that reports `DS9001` instead.

use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};

use datascript_diagnostics::codes::DS9001;
use datascript_diagnostics::{Diagnostic, FileId};
use futures::FutureExt;

/// Runs `f`, turning a panic into `DS9001`. `what` says what was being done, as in "checking
/// the program", and `file` names the file the diagnostic belongs to.
///
/// Callers must not reuse state that `f` may have left half-updated; the session's only shared
/// state is its schema cache, which `f` updates with single inserts.
pub fn catch_panic<T>(
    file: Option<FileId>,
    what: &str,
    f: impl FnOnce() -> T,
) -> Result<T, Diagnostic> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| panic_diagnostic(file, what, &*payload))
}

/// [`catch_panic`] for a future: a panic while it is polled becomes `DS9001`.
pub async fn catch_panic_async<F: Future>(
    file: Option<FileId>,
    what: &str,
    future: F,
) -> Result<F::Output, Diagnostic> {
    AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .map_err(|payload| panic_diagnostic(file, what, &*payload))
}

fn panic_diagnostic(file: Option<FileId>, what: &str, payload: &(dyn Any + Send)) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        DS9001,
        format!("internal error: DataScript crashed while {what}"),
    );
    if let Some(file) = file {
        diagnostic = diagnostic.in_file(file);
    }
    if let Some(message) = panic_message(payload) {
        diagnostic = diagnostic.note(format!("the crash said: {message}"));
    }
    diagnostic.note("this is a bug in DataScript; please report it")
}

/// The first line of a panic's message, if it has one.
fn panic_message(payload: &(dyn Any + Send)) -> Option<&str> {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))?;
    message.lines().next().filter(|line| !line.is_empty())
}
