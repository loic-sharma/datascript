//! `datascript/run` prints exactly what `ds run` prints (M3-05, D-054).
//!
//! This lives with the CLI because it runs both through the `ds` binary: `ds lsp` over stdio,
//! and `ds run --color always` (the renderer `ds run` uses on a color terminal, as VS Code's
//! terminal is). `ds run` writes diagnostics to standard error and tables to standard output;
//! the server returns them in the order they are written.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// A warning, a chart, a table cut by `--max-rows`, and a runtime error at the end.
const PROGRAM: &str = "from 'orders.csv' | sort by orderId | take 1;\n\
                       from 'orders.csv' | project customerId, amount | render barchart;\n\
                       from 'big.csv' | extend y = x * 1000000000000000\n";

const ORDERS: &str = "orderId,customerId,amount\n1,c-1,10.5\n2,c-2,20\n3,c-3,7\n";

struct Server {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start(root: &Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ds"))
            .arg("lsp")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut server = Server {
            child,
            input,
            output,
            next_id: 0,
        };
        let root = uri_of(root);
        server.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": root,
                "workspaceFolders": [{ "uri": root, "name": "w" }],
                "capabilities": {}
            }),
        );
        server.notify("initialized", json!({}));
        server
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.input.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn receive(&mut self) -> Value {
        let mut length = None;
        loop {
            let mut line = String::new();
            assert!(
                self.output.read_line(&mut line).unwrap() > 0,
                "server exited"
            );
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                length = Some(n.parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.unwrap()];
        self.output.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive();
            if message["id"] == id && message.get("method").is_none() {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn uri_of(path: &Path) -> String {
    let path = path.canonicalize().unwrap();
    let mut uri = String::from("file://");
    for part in path.to_string_lossy().split('/').filter(|p| !p.is_empty()) {
        uri.push('/');
        for b in part.bytes() {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                uri.push(b as char);
            } else {
                uri.push_str(&format!("%{b:02X}"));
            }
        }
    }
    uri
}

/// `ds run --color always <args>` in `dir`: (stdout, stderr).
fn ds_run(dir: &Path, args: &[&str]) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(dir)
        .args(["--color", "always", "run"])
        .args(args)
        .output()
        .unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// `lsp` is `stdout` written into `stderr` at one place.
fn assert_interleaved(lsp: &str, stdout: &str, stderr: &str) {
    assert!(!stdout.is_empty());
    let at = lsp
        .find(stdout)
        .unwrap_or_else(|| panic!("the tables differ:\n{lsp}\n---\n{stdout}"));
    let rest = format!("{}{}", &lsp[..at], &lsp[at + stdout.len()..]);
    assert_eq!(rest, stderr, "the diagnostics differ");
}

#[test]
#[cfg_attr(windows, ignore = "the test builds file URIs for Unix paths")]
fn run_output_equals_ds_run() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("orders.csv"), ORDERS).unwrap();
    std::fs::write(dir.path().join("big.csv"), "x\n1\n10000\n").unwrap();
    std::fs::write(dir.path().join("report.ds"), PROGRAM).unwrap();
    let mut server = Server::start(dir.path());
    let uri = format!("{}/report.ds", uri_of(dir.path()));
    server.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "datascript", "version": 1, "text": PROGRAM } }),
    );

    let cases: [(Value, Vec<&str>); 3] = [
        (json!({}), vec!["report.ds"]),
        (
            json!({ "maxRows": 1 }),
            vec!["report.ds", "--max-rows", "1"],
        ),
        (
            json!({ "statement": 2 }),
            vec!["report.ds", "--statement", "2"],
        ),
    ];
    for (extra, args) in cases {
        let mut params = json!({ "textDocument": { "uri": uri }, "color": true });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let result = server.request("datascript/run", params);
        let (stdout, stderr) = ds_run(dir.path(), &args);
        let output = result["output"].as_str().unwrap();
        assert!(output.contains('\x1b'), "colored: {output:?}");
        assert_interleaved(output, &stdout, &stderr);
        // The program shows every kind of output it is meant to.
        assert!(stderr.contains("DS5001"), "{stderr}");
        if args.len() < 3 || args[1] == "--max-rows" {
            assert!(
                stderr.contains("DS2103") && stderr.contains("DS4003"),
                "{stderr}"
            );
        }
        if args.get(1) == Some(&"--max-rows") {
            assert!(stdout.contains("… 2 more rows"), "{stdout}");
        }
        // Only the third statement fails; `ds run` exits with 1 then.
        assert_eq!(result["failed"], stderr.contains("DS4003"), "{args:?}");
    }
}
