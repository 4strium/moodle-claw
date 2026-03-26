# Moodle-Claw

A command-line tool and AI agent skill for interacting with Moodle LMS, based on [Edu Sync](https://github.com/mkroening/edu-sync).

[![CI](https://github.com/mkroening/edu-sync/actions/workflows/ci.yml/badge.svg)](https://github.com/mkroening/edu-sync/actions/workflows/ci.yml)

## Overview

Moodle-Claw extends the original Edu Sync project to provide:
- **Agent-friendly CLI** with JSON/Markdown output for AI integration
- **SSO authentication** support for university single sign-on
- **Course browsing** and content search capabilities
- **On-demand file downloading** with intelligent caching
- **OpenClaw skill** for natural language interaction with Moodle content

It accesses Moodle via the mobile web services API (same as the official Moodle app).

## Installation

```bash
# Build from source
cargo build --release

# Install to ~/.local/bin
cp target/release/moodle-claw ~/.local/bin/
```

## Quick Start

### 1. Configure your Moodle account

```bash
moodle-claw configure
```

This will interactively prompt for:
- Moodle server URL
- Authentication method (Token, SSO, or Username/Password)
- Download path

### 2. List your courses

```bash
moodle-claw courses --refresh
```

### 3. Browse course content

```bash
moodle-claw content "Course Name"
```

### 4. Download files

```bash
moodle-claw get "Course/Section/file.pdf"
```

### 5. Sync a course (download all files)

```bash
moodle-claw sync "Course Name"
```

## Authentication Methods

### Direct Token
```bash
moodle-claw configure --url https://moodle.example.com --token YOUR_TOKEN --path ~/Moodle
```

### SSO (Single Sign-On)
For universities using SSO:

1. Log into Moodle in your browser
2. Open developer console (F12) → Network tab
3. Visit: `https://YOUR_MOODLE/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport=12345&urlscheme=moodlemobile`
4. Copy the failed request's link address (`moodlemobile://token=...`)
5. Configure:
   ```bash
   moodle-claw configure --url https://moodle.example.com --sso-url "moodlemobile://token=..." --path ~/Moodle
   ```

### Username/Password
```bash
moodle-claw configure --url https://moodle.example.com --username user --password pass --path ~/Moodle
```

## Commands

| Command | Description |
|---------|-------------|
| `moodle-claw configure` | Set up Moodle connection |
| `moodle-claw status` | Show configuration status |
| `moodle-claw courses` | List enrolled courses |
| `moodle-claw content <course>` | Show course structure |
| `moodle-claw search <query>` | Search across courses |
| `moodle-claw get <path>` | Download a specific file |
| `moodle-claw sync [course]` | Sync course files locally |

All commands support `--output json` for machine-readable output.

## OpenClaw Skill

Moodle-Claw includes a `SKILL.md` file for use with OpenClaw agents. This allows natural language queries like:
- "Explique-moi le cours de mécanique"
- "Quel est l'exercice 3 du TD2 en maths?"
- "Télécharge tous les fichiers du cours de physique"

## Project Structure

This project is based on the [Edu Sync](https://github.com/mkroening/edu-sync) Rust workspace:

| Crate | Description |
|-------|-------------|
| `edu-sync-cli` | CLI application (`moodle-claw` binary) |
| `edu-sync` | Core synchronization library |
| `edu-ws` | Moodle web services API wrapper |
| `edu-ws-derive` | Procedural macros |

## Shell Completions

```bash
COMPLETE=<SHELL> moodle-claw
```

Where `<SHELL>` is one of `bash`, `elvish`, `fish`, `powershell`, or `zsh`.

## License

This project is licensed under the GNU General Public License v3.0 only ([LICENSE](LICENSE)).

## Trademark Notice

Moodle™ is a [registered trademark](https://moodle.com/trademarks/) of Moodle Pty Ltd. Moodle-Claw is not sponsored, endorsed, licensed by, or affiliated with Moodle Pty Ltd.

## Credits

Based on [Edu Sync](https://github.com/mkroening/edu-sync) by Martin Kröning.
