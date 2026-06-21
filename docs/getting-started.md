# Getting started

## The REPL

surd is written in Rust; a [Rust toolchain](https://rustup.rs) is the only
requirement.

```sh
cargo run            # interactive REPL
echo "sqrt(2)^2" | cargo run    # pipe mode
cargo test           # the test suite
```

The REPL reads statements and prints the value of each:

```text
>> 1/3 + 1/6
1/2
>> fact(n) := if n == 0 then 1 else n*fact(n-1) end
<function(n)>
>> fact(20)
2432902008176640000
```

Input that opens a block or bracket keeps reading (with a `..` continuation
prompt) until the block closes — `if`/`while`/`function` blocks end with
`end`, and a newline inside `(...)` or `[...]` is just line continuation.

Meta-commands:

| Command | Effect |
| --- | --- |
| `:vars` | List every binding in the workspace |
| `:q` / `:quit` | Quit (Ctrl-D also works) |

Ctrl-C cancels the entry being typed.

## The web app

The quickest way to try surd is the hosted web app — nothing to install:

**<https://dariusjlukas.github.io/surd/>**

The full frontend — notebook cells, KaTeX-rendered math, interactive 2D/3D
plots, a workspace panel, data import/export — lives in `app/` and runs the
same engine compiled to WebAssembly. To run it locally:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack            # if you don't have it
wasm-pack build wasm --target web --out-dir ../app/src/engine/pkg
cd app && npm install && npm run dev      # → http://localhost:5173
```

In the web app, [`plot(...)`](reference/plotting.md) results are drawn as
interactive plots that resample at full resolution as you pan and zoom.

## The desktop app (offline)

The same frontend also ships as a fully offline desktop app, built with
[Tauri](https://tauri.app/): the engine runs as WebAssembly inside the
operating system's own webview, so nothing talks to a server and the app — its
documentation included, behind the in-app **Help** button — works with no
network at all. Installers for macOS, Windows, and Linux are attached to each
[GitHub release](https://github.com/dariusjlukas/surd/releases).

To build it yourself (a [Rust toolchain](https://rustup.rs) and Node are the
only requirements):

```sh
cd app
npm install
npm run tauri:dev      # dev window with hot reload (builds the wasm engine first)
npm run tauri:build    # native installer in app/src-tauri/target/release/bundle/
```

surd follows [semantic versioning](https://semver.org/), with one version shared
by the engine, the wasm binding, and the app. It's surfaced wherever you'd look:
`surd --version` and the REPL banner on the CLI, and **Settings → About** in the
app; [`CHANGELOG.md`](https://github.com/dariusjlukas/surd/blob/main/CHANGELOG.md)
records what changed in each release.

## A two-minute tour

```text
>> 1.5                       # decimals are exact rationals
3/2
>> x := 3                    # := assigns
3
>> x^2 + 1
10
>> diff(x^2, x)              # diff takes x by name: 2x, evaluated at x = 3
6
>> A := [1, 2; 3, 4]         # matrices: , between entries, ; between rows
[ 1  2 ]
[ 3  4 ]
>> A * inv(A)                # exactly the identity, not "approximately"
[ 1  0 ]
[ 0  1 ]
>> (1 + I)^2                 # complex numbers fold eagerly
2*I
>> N(sqrt(2), 10)            # floats are opt-in, to any precision
1.414213562
```

Read on: [Syntax](language/syntax.md) covers the grammar in full.
