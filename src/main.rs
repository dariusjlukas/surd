//! REPL harness for the `surd` engine.
//!
//! Both interactive and piped input accumulate lines until the program is
//! syntactically complete (balanced brackets and `if`/`while`/`function` …
//! `end`), then evaluate the whole unit and print its value. Interactive mode
//! adds line editing, history, and a continuation prompt.

use std::io::IsTerminal;
use surd::lexer::{is_blank, is_incomplete};
use surd::Interpreter;

fn main() {
    // Run the whole REPL on a large-stack thread so deep recursion/nesting hits
    // the evaluator's depth guards (graceful errors) rather than the OS stack.
    surd::run_with_stack(|| {
        let mut interp = Interpreter::new();
        if std::io::stdin().is_terminal() {
            banner();
            run_interactive(&mut interp);
        } else {
            run_pipe(&mut interp);
        }
    });
}

fn banner() {
    println!("surd — an exact-by-default CAS REPL (prototype)");
    println!("  ':=' assigns, '=' is an equation, 'N(x)' goes to float.");
    println!("  if/while/function are blocks ended with 'end'.");
    println!("  try:  fact(n) := if n == 0 then 1 else n*fact(n-1) end   then   fact(20)");
    println!("  ':vars' lists the workspace, ':q' quits.");
    println!();
}

fn run_interactive(interp: &mut Interpreter) {
    use rustyline::error::ReadlineError;
    let mut rl = match rustyline::DefaultEditor::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("could not start line editor: {e}");
            return;
        }
    };
    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() { ">> " } else { ".. " };
        match rl.readline(prompt) {
            Ok(line) => {
                if buffer.is_empty() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == ":q" || trimmed == ":quit" {
                        break;
                    }
                    if trimmed == ":vars" {
                        print_workspace(interp);
                        continue;
                    }
                }
                if !buffer.is_empty() {
                    buffer.push('\n');
                }
                buffer.push_str(&line);
                if is_incomplete(&buffer) {
                    continue; // keep reading the block
                }
                if !is_blank(&buffer) {
                    let _ = rl.add_history_entry(buffer.as_str());
                    report(interp, &buffer);
                }
                buffer.clear();
            }
            Err(ReadlineError::Interrupted) => buffer.clear(), // Ctrl-C cancels the entry
            Err(ReadlineError::Eof) => break,                  // Ctrl-D exits
            Err(e) => {
                eprintln!("input error: {e}");
                break;
            }
        }
    }
}

fn run_pipe(interp: &mut Interpreter) {
    use std::io::BufRead;
    let mut buffer = String::new();
    for line in std::io::stdin().lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if buffer.is_empty() && line.trim().is_empty() {
            continue;
        }
        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(&line);
        if is_incomplete(&buffer) {
            continue;
        }
        if !is_blank(&buffer) {
            report(interp, &buffer);
        }
        buffer.clear();
    }
    if !is_blank(&buffer) {
        report(interp, &buffer);
    }
}

fn report(interp: &mut Interpreter, src: &str) {
    match interp.eval_line(src) {
        Ok(value) => println!("{}", value),
        Err(err) => println!("error: {}", err),
    }
}

fn print_workspace(interp: &Interpreter) {
    let mut vars: Vec<_> = interp.workspace().collect();
    vars.sort_by(|a, b| a.0.cmp(b.0));
    if vars.is_empty() {
        println!("(workspace empty)");
    }
    for (name, value) in vars {
        println!("  {} = {}", name, value);
    }
}
