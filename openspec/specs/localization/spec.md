# Localization Specification

## Purpose

Defines the es/en string catalogue behind `format.rs`, session-locale resolution order, fallback
to English, and the requirement that this catalogue never reaches the sudoers rule header. (PRD
NFR "Idioma" row)

## Requirements

### Requirement: Catalogue Behind format.rs, Call Sites Unchanged

Every user-facing string `format.rs` renders (menu labels, tooltip, status line, notification
text) MUST route through an es/en catalogue backing `format.rs`'s existing public signatures. No
call site outside `format.rs` MUST change to accommodate localization.

#### Scenario: A Spanish locale changes format.rs output without touching callers
- GIVEN the resolved locale is Spanish
- WHEN `format::status_line`, `format::tooltip`, and `format::toggle_label` are called with the
  same arguments as under English
- THEN each returns its Spanish rendering, and no caller in `tray.rs` or `app.rs` required any
  change to receive it
- Testable via: `cargo test`

### Requirement: Locale Resolution Order With English Fallback

The default UI locale MUST be resolved from `LC_ALL`, then `LC_MESSAGES`, then `LANG`, in that
order; the first of these set to a Spanish locale value selects Spanish, and any other outcome
(none set, or set to a non-Spanish value) selects English.

| `LC_ALL` | `LC_MESSAGES` | `LANG` | Resolved |
|---|---|---|---|
| `es_ES.UTF-8` | (any) | (any) | Spanish |
| unset | `es_ES.UTF-8` | (any) | Spanish |
| unset | unset | `es_ES.UTF-8` | Spanish |
| unset | unset | unset | English |
| `en_US.UTF-8` | `es_ES.UTF-8` | (any) | English |

#### Scenario: LC_ALL takes precedence over LC_MESSAGES and LANG
- GIVEN `LC_ALL=en_US.UTF-8`, `LC_MESSAGES=es_ES.UTF-8`, `LANG=es_ES.UTF-8`
- WHEN the locale is resolved
- THEN it resolves to English
- Testable via: `cargo test`

#### Scenario: No locale variable set resolves to English
- GIVEN `LC_ALL`, `LC_MESSAGES`, and `LANG` are all unset
- WHEN the locale is resolved
- THEN it resolves to English
- Testable via: `cargo test`

### Requirement: The Sudoers Header Is Never Reachable From This Catalogue

`format.rs`'s catalogue MUST have no code path that reaches the sudoers rule template
(`nopass-core`'s header rendering); the header's fixed English text MUST render byte-identically
regardless of the resolved UI locale.

#### Scenario: The rendered header is byte-identical under a Spanish locale
- GIVEN the resolved UI locale is Spanish
- WHEN a sudoers rule header is rendered
- THEN its bytes are identical to the header rendered under English
- Testable via: `cargo test`

### Requirement: polkit's Own Dialog Localization Is Not Reimplemented

The system MUST NOT implement its own polkit authentication-dialog localization. The installed
`data/com.enfoquestic.nopass.policy` MUST provide an `xml:lang="es"` variant of its
`<description>` and `<message>` elements so polkit's own mechanism resolves the caller's locale
without application code.

#### Scenario: The installed policy carries a Spanish description and message
- GIVEN `data/com.enfoquestic.nopass.policy`
- WHEN the file is parsed
- THEN it contains an `xml:lang="es"` variant of both `<description>` and `<message>`
- Testable via: `cargo test` (XML fixture parse of the static data file)
