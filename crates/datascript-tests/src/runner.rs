//! Discovers `.dstest` files and runs each as its own libtest trial.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use datascript_diagnostics::did_you_mean;
use libtest_mimic::{Arguments, Failed, Trial};

use crate::bless::bless;
use crate::case::{Order, TestCase};
use crate::driver::{CaseInput, Driver, DriverConfig, DriverError, dispatch};
use crate::output::{canonicalize, diff, normalize_paths};

/// The drivers available to the harness, one per engine name.
#[derive(Clone)]
pub struct Drivers {
    drivers: Vec<Arc<dyn Driver>>,
    config: DriverConfig,
}

impl std::fmt::Debug for Drivers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drivers")
            .field(
                "engines",
                &self.drivers.iter().map(|d| d.engine()).collect::<Vec<_>>(),
            )
            .field("config", &self.config)
            .finish()
    }
}

impl Drivers {
    pub fn new(config: DriverConfig) -> Self {
        Self {
            drivers: Vec::new(),
            config,
        }
    }

    /// Adds a driver. A later driver for the same engine replaces an earlier one.
    pub fn with(mut self, driver: impl Driver + 'static) -> Self {
        self.drivers.retain(|d| d.engine() != driver.engine());
        self.drivers.push(Arc::new(driver));
        self
    }

    /// The drivers that exist today: the [`PipelineDriver`](crate::PipelineDriver), which supports
    /// only the modes whose pipeline stages exist.
    pub fn current() -> Self {
        Self::new(DriverConfig::default())
            .with(crate::PipelineDriver::new(crate::case::DEFAULT_ENGINE))
    }

    fn get(&self, engine: &str) -> Option<Arc<dyn Driver>> {
        self.drivers.iter().find(|d| d.engine() == engine).cloned()
    }
}

/// The repository root (two levels above this crate).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// The golden-test entry point: discovers cases under `tests/cases/` and runs them.
pub fn main(drivers: Drivers) -> ExitCode {
    let args = Arguments::from_args();
    let root = repo_root();
    let trials = trials(
        &root.join("tests").join("cases"),
        &root,
        &drivers,
        bless_enabled(),
    );
    libtest_mimic::run(&args, trials).exit_code()
}

fn bless_enabled() -> bool {
    std::env::var_os("DS_BLESS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Every `.dstest` file under `dir`, sorted.
pub fn discover(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "dstest") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// One trial per case file, named by its path relative to `cases_dir` (so `-- join/` filters
/// by directory) and tagged with its mode.
pub fn trials(cases_dir: &Path, repo_root: &Path, drivers: &Drivers, bless: bool) -> Vec<Trial> {
    discover(cases_dir)
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(cases_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            trial(name, path, repo_root, drivers, bless)
        })
        .collect()
}

fn trial(name: String, path: PathBuf, repo_root: &Path, drivers: &Drivers, bless: bool) -> Trial {
    let prepared = prepare(path, repo_root, drivers, bless);
    let ignored = prepared.ignored;
    let kind = prepared.kind;
    Trial::test(name, move || (prepared.run)().map_err(Failed::from))
        .with_kind(kind)
        .with_ignored_flag(ignored)
}

/// A case ready to run: what libtest needs to know up front, and the work itself.
pub struct Prepared {
    /// The case's mode, or `invalid` for a file that could not be parsed.
    pub kind: &'static str,
    /// Skipped, or some engine does not support the mode yet.
    pub ignored: bool,
    /// Runs the case. The error is the failure message.
    pub run: Box<dyn FnOnce() -> Result<(), String> + Send>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared")
            .field("kind", &self.kind)
            .field("ignored", &self.ignored)
            .finish_non_exhaustive()
    }
}

/// Reads and parses one case file and decides how it runs.
pub fn prepare(path: PathBuf, repo_root: &Path, drivers: &Drivers, bless: bool) -> Prepared {
    let display = path
        .strip_prefix(repo_root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    let fail = |message: String| Prepared {
        kind: "invalid",
        ignored: false,
        run: Box::new(move || Err(message)),
    };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => return fail(format!("{display}: cannot read file: {e}")),
    };
    let case = match TestCase::parse(&text) {
        Ok(case) => case,
        Err(e) => return fail(format!("{display}:{}: {}", e.line, e.message)),
    };

    let mut engines = Vec::new();
    for engine in &case.engines.value {
        match drivers.get(engine) {
            Some(driver) => engines.push(driver),
            None => {
                let known: Vec<&str> = drivers.drivers.iter().map(|d| d.engine()).collect();
                let hint = match did_you_mean(engine, known.iter().copied()).single() {
                    Some(k) => format!(" (did you mean `{k}`?)"),
                    None => format!(" (known engines: {})", known.join(", ")),
                };
                return fail(format!(
                    "{display}:{}: unknown engine `{engine}`{hint}",
                    case.engines.line.max(1)
                ));
            }
        }
    }

    let kind = case.mode.as_str();
    // Ignored cases still fail when forced with `--ignored`: they never count as passed.
    let reason = match (&case.skip, engines.iter().find(|d| !d.supports(case.mode))) {
        (Some(reason), _) => Some(format!("skipped: {reason}")),
        (None, Some(d)) => Some(format!(
            "mode `{}` is not implemented for engine `{}` yet",
            case.mode,
            d.engine()
        )),
        (None, None) => None,
    };
    if let Some(reason) = reason {
        return Prepared {
            kind,
            ignored: true,
            run: Box::new(move || Err(format!("{display}: {reason}"))),
        };
    }

    let config = drivers.config.clone();
    let repo_root = repo_root.to_path_buf();
    Prepared {
        kind,
        ignored: false,
        run: Box::new(move || {
            run_case(
                &display, &path, &text, &case, &engines, &config, &repo_root, bless,
            )
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_case(
    display: &str,
    path: &Path,
    text: &str,
    case: &TestCase,
    engines: &[Arc<dyn Driver>],
    config: &DriverConfig,
    repo_root: &Path,
    bless_mode: bool,
) -> Result<(), String> {
    let input = CaseInput {
        path,
        query: &case.query,
        now_micros: case.now_micros,
        tables: &case.tables,
        schema: case.schema.as_ref(),
        config,
    };
    let test_dir = path.parent().unwrap_or(Path::new("."));

    let order = case.order.unwrap_or_else(|| {
        let defined = engines
            .first()
            .and_then(|d| d.output_order_defined(&case.query));
        if defined == Some(true) {
            Order::Strict
        } else {
            Order::Any
        }
    });

    let mut outputs = Vec::new();
    for driver in engines {
        let output = match dispatch(driver.as_ref(), case.mode, &input) {
            Ok(output) => output,
            Err(DriverError::NotImplemented) => {
                return Err(format!(
                    "{display}: engine `{}` said it supports mode `{}` but returned \"not implemented\"",
                    driver.engine(),
                    case.mode
                ));
            }
            Err(DriverError::Failed(message)) => {
                return Err(format!(
                    "{display}: engine `{}`: {message}",
                    driver.engine()
                ));
            }
        };
        outputs.push((
            driver.engine().to_string(),
            normalize_paths(&output, test_dir, repo_root),
        ));
    }

    if bless_mode {
        if let Some(new_text) = bless(text, case, &outputs, order) {
            fs::write(path, new_text)
                .map_err(|e| format!("{display}: cannot write blessed file: {e}"))?;
        }
        return Ok(());
    }

    let mut failures = Vec::new();
    for (engine, output) in &outputs {
        let label = if outputs.len() > 1 {
            format!(" [{engine}]")
        } else {
            String::new()
        };
        let Some(expected) = case.expected_for(engine) else {
            failures.push(format!(
                "{display}{label}: no `---- expected` section; run with DS_BLESS=1 to create it. \
                 Actual output:\n{output}"
            ));
            continue;
        };
        let (want, got) = (
            canonicalize(&expected.body, order),
            canonicalize(output, order),
        );
        if want != got {
            let reordered = |s: &str| canonicalize(s, Order::Strict) != canonicalize(s, order);
            let note = if reordered(&expected.body) || reordered(output) {
                " (rows sorted, `order: any`)"
            } else {
                ""
            };
            failures.push(format!(
                "{display}:{}{label}: output does not match{note}\n{}",
                expected.line,
                diff(&want, &got)
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        failures.push("run with DS_BLESS=1 to accept the actual output".to_string());
        Err(failures.join("\n"))
    }
}
