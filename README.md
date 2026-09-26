# Claude Usage Checker

A tiny Tauri app that shows your weekly Claude Code usage as a bar. The fill is how much of your weekly quota you've used; the vertical line is how far through the 7-day window you are. The bar turns red when usage is ahead of the line.

## Token location

On launch the app looks for Claude Code's OAuth credentials in the standard place:

| OS | Location |
|---|---|
| Windows | `%USERPROFILE%\.claude\.credentials.json` |
| macOS | Login Keychain item `Claude Code-credentials` (or `~/.claude/.credentials.json` if Claude Code fell back to a file) |
| Linux | `~/.claude/.credentials.json` |

`$CLAUDE_CONFIG_DIR` overrides `~/.claude`. If a file is found, its path fills the textbox. If none is found, the box stays blank, and on macOS the app reads the Keychain instead (macOS will ask you to allow access). You can type a different path, or click **Find token path** to pick the file. On macOS, press <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>.</kbd> in the picker to show hidden files.

When the access token has expired, the app refreshes it the same way Claude Code does and writes the new tokens back. Refresh tokens can only be used once, so Claude Code keeps working afterwards. Usage is polled every 5 minutes, and the time line moves every 30 seconds.

## Build

```bash
npm install
npx tauri dev                 # run locally
npx tauri build --no-bundle   # portable binary, no installer
```

The app is portable, so no installer is needed:

- **Windows:** run `Claude Usage Checker.exe` from anywhere. It needs the WebView2 runtime, which ships with Windows 10 and 11.
- **macOS:** unzip `Claude Usage Checker-macos.zip` and run the `.app`. It isn't notarized, so the first time, right-click it and choose **Open**.

Cross-compile the portable Windows exe from Linux (needs `lld`, `llvm`, and `cargo install cargo-xwin`):

```bash
npx tauri build --no-bundle --runner cargo-xwin --target x86_64-pc-windows-msvc
```

macOS builds need a Mac. `.github/workflows/build.yml` builds both portable versions on GitHub Actions.
