# owa-calendar
OWA calendar with notifications for Linux

## About

This application provides access to Outlook Web Access (OWA) / Exchange calendar on Linux with desktop notifications.

## Configuration

On first run the app creates `config.toml` (in `~/.config/owa-calendar/` on
Linux) and opens it in your editor. Fill in at least:

- `host` — OWA base URL, e.g. `https://owa.example.com/`
- `username` — account login, with domain if required: `DOMAIN\username`

The **password is not stored in the config file**. It is entered in the
in-app login dialog and saved to the operating system's secret store
(Secret Service / GNOME Keyring / KWallet on Linux, Keychain on macOS,
Credential Manager on Windows). The dialog pops up automatically when no
password is stored or the stored one is rejected, and can be reopened any
time via the **Credentials** button. Submitting it also writes the entered
username back to `config.toml`.

## Security & Privacy

This is open-source software that you can audit and build yourself. The application:
- Runs entirely locally on your machine
- Does not send any data to third parties
- Stores your password only in the OS secret store, never on disk in plain text
