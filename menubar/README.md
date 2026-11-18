# Portman menu bar app

A macOS menu bar app on top of the `portman` CLI. It keeps a few fixed addresses
("slots", by default `localhost:5001` and `localhost:5002`) and lets you point each
one at any project with a click, without restarting servers. It is built for
previewing several git worktrees of one repository side by side when the app's
login only trusts fixed localhost addresses.

## What it does

- Lists every portman project with its port, branch and whether it is running.
- Finds all git worktrees of the registered repositories and registers them itself;
  projects whose folder is gone are removed.
- Links or unlinks a slot to a project (`portman link` / `portman unlink`).
- Starts and stops a project's dev server, with the output kept in a log file.

## Requirements

- macOS, Rust, and Caddy running as a service (`brew services start caddy`).
- The `portman` CLI at `/opt/homebrew/bin/portman` (`cargo build --release` in the
  repository root, then copy `target/release/portman` there).

## Build and install

```sh
./bundle.sh
open ~/Applications/Portman.app
```

## Settings

`~/Library/Application Support/com.canac.portman/menubar.toml`, created on first
launch and opened by **Edit Settings…**:

```toml
slots = [5001, 5002]

[[commands]]
repo = "my-org/my-repo"            # matched against the git origin URL; omit to match all
run = "npm run dev -- --port $PORT" # runs in the project folder with $PORT set
```

Pick a port range for projects that avoids the slots, in portman's own
`config.toml` (for example `ranges = [[5100, 5999]]`).

## Limits

- A project can hold one slot at a time, so two slots need two projects.
- Servers started by the app are stopped when the app quits.
- Paths to `portman` and Homebrew are fixed to `/opt/homebrew`.
