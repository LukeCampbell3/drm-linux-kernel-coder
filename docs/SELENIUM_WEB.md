# Selenium web capability

`web.selenium` gives a DRM episode a real, JavaScript-capable browser observation. It opens the absolute URL in the episode's `--url`, extracts the final URL, title, and visible body text as JSON, and threads that JSON into the next capability.

Web access is opt-in. The daemon must have `DRMD_WEB_ALLOWED_HOSTS`, as a comma-separated list of exact hosts or `*.example.com` wildcards. `*` permits every public host. Loopback, link-local, and private IP literals remain blocked unless `DRMD_WEB_ALLOW_PRIVATE=1` is also set.

```bash
export DRMD_WEB_ALLOWED_HOSTS='docs.rs,*.rust-lang.org'
drmd serve --socket /tmp/drmd.sock --work /tmp/drmd-work --state /tmp/drmd-state

drmd submit --socket /tmp/drmd.sock \
  --task rust_docs \
  --ops web.selenium,transform.summarize,fs.write \
  --url https://www.rust-lang.org/learn \
  --output outputs/rust-docs.txt
```

The desktop image installs Chromium, ChromeDriver, Python Selenium, and the bridge automatically. For a remote Selenium Grid, set `DRMD_WEBDRIVER_URL`. Other deployments can override `DRMD_SELENIUM_BRIDGE` and `DRMD_WEB_PYTHON`.

Controls:

- `DRMD_WEB_TIMEOUT_SECS` defaults to 20.
- `DRMD_WEB_MAX_OUTPUT_BYTES` defaults to 1,000,000.
- URLs with credentials and non-HTTP(S) schemes are rejected before Selenium starts.
- Each request gets a fresh browser session; cookies and local storage are not shared between DRM applications.
