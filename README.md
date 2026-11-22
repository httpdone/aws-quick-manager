# AWS Quick Manager

Account manager and switcher for the Amazon Quick desktop app (Windows and macOS).

The desktop app already keeps several accounts side by side — one entry per
account in `~/.quickwork/profiles.json`, a data folder under
`~/.quickwork/profiles/<id>/`, and the session in the OS credential store
(Windows Credential Manager / macOS Keychain, service
`quickwork-<provider>-<id>`, user `session`) — but its UI has no way to switch
between them. This app reads that state directly and switches the account the
desktop starts with.

Status: early. The account list works; adding, switching and quota are next.

## Develop

Requires Node ≥ 22, pnpm and Rust (stable).

```bash
pnpm install
pnpm tauri dev
cd src-tauri && cargo test
```

Unofficial; not affiliated with Amazon Web Services.

## License

MIT
