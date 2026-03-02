# browser-cat

A Rust port of [bcat](https://github.com/rtomayko/bcat) by Ryan Tomayko — pipe text or HTML to your browser from the command line.

```sh
# plain text
echo "hello world" | browser-cat

# ANSI colours
cargo build 2>&1 | browser-cat --ansi

# an HTML file
browser-cat index.html

# a command's output
browser-cat --command -- make docs
```

## Binaries

| Binary | Description |
|--------|-------------|
| `browser-cat` | Main utility — opens a browser window and streams output into it |
| `a2h` | Standalone ANSI-to-HTML filter, writes to stdout |

## Options

```
Usage: browser-cat [OPTIONS] [FILE]...

Options:
  -H, --html              Input is already HTML (passthrough)
  -t, --text              Input is plain text (wrap in <pre>)
  -b, --browser <BROWSER> Open this browser instead of the system default
  -T, --title <TEXT>      Browser window title
  -a, --ansi              Convert ANSI escape sequences to HTML
  -p, --persist           Keep server running after browser closes (allow reload)
  -c, --command           Treat arguments as a command to run; read its stdout
  -d, --debug             Verbose debug logging to stderr
  -h, --help              Print help
```

The browser can also be set via the `BCAT_BROWSER` environment variable.

## btee mode

Symlink (or copy) the binary as `btee` and it will tee raw input to stdout
while also opening a browser window — useful for watching long-running builds:

```sh
ln -s browser-cat btee
make 2>&1 | btee | grep error
```

## Format detection

If no format flag is given, `browser-cat` sniffs the first chunk of input: if
the first non-whitespace character is `<` it is treated as HTML, otherwise as
plain text wrapped in `<pre>`.

## Building

```sh
cargo build --release
# binaries land in target/release/browser-cat and target/release/a2h
```

Requires Rust 1.80+ (uses `OnceLock` and `axum` 0.8).

## Credits

Based on [bcat](https://github.com/rtomayko/bcat) by Ryan Tomayko, originally
written in Ruby. See [LICENSE](LICENSE) for copyright details.
