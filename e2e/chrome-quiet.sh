#!/bin/sh
# Launches the installed Google Chrome with stdout/stderr sent to /dev/null (macOS).
#
# Why: when Chrome starts it can spawn Google's updater ("GoogleUpdater --wake-all") and a crashpad handler. Those
# helpers inherit Chrome's stdout/stderr, which Playwright connects to pipes, and they outlive the browser by
# minutes. A Playwright worker cannot exit while a pipe it created is still open, so `playwright test` printed all
# its results and then hung. Chrome talks to Playwright over --remote-debugging-pipe (fds 3 and 4), which `exec`
# keeps, so redirecting the two log streams loses nothing but the browser's own console noise.
exec "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" "$@" >/dev/null 2>&1
