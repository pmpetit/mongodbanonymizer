# Masking Methods

`manon` ships nine masking methods.  The appropriate method for each field is
chosen automatically based on the field-name category looked up in
`identifier_category.csv`, but you can override it by editing the YAML schema
file directly.

---

## PRESERVE_TOKEN

Replaces each character according to its class while preserving structure:

- Uppercase letter → different uppercase letter (deterministic)
- Lowercase letter → different lowercase letter
- Digit → different digit
- Separator (space, `-`, `_`, `.`, …) → kept as-is

**Use for**: names, identifiers, codes — anything where the shape of the
value matters for downstream processing.

**Example**: `John-Doe` → `Mnbq-Yjf`

---

## REDACT_ALPHANUMERIC

Replaces every letter with `X` and every digit with `0`.  Separators are kept.

**Use for**: free-text fields or codes where only the length and punctuation
pattern need to be preserved.

**Example**: `AB-1234` → `XX-0000`

---

## MASK_CONTACT_URI

Handles three contact-URI shapes:

- **E-mail** — the local part (before `@`) is replaced; the domain is kept.
- **URL** — the path segments and query-string values are replaced; the
  scheme and host are kept.
- **Phone number** — digits are replaced deterministically; non-digit
  separators are kept.

**Example (email)**: `alice@example.com` → `xyzw@example.com`

---

## MASK_NETWORK_ID

Anonymizes network identifiers segment by segment:

- **IPv4** — each octet replaced independently.
- **IPv6** — each group replaced independently.
- **MAC address** — each byte replaced independently; the OUI (first 3
  bytes) may be kept depending on configuration.

**Example (IPv4)**: `192.168.1.42` → `10.74.253.17`

---

## GENERALIZE_LOCATION

Truncates a postal code or geographic area code to its first 2–3 characters,
generalizing the location to a region rather than a precise address.

**Example**: `75013` → `75`

---

## NOISY_DATE

Shifts a date value by a deterministic noise of up to ±30 days, derived from
the input value itself.  The year is preserved.

**Use for**: timestamps, birth dates, booking dates — cases where the exact
date is sensitive but the approximate period is needed for analysis.

**Example**: `2024-06-15` → `2024-06-02`

---

## NOISY_POSITION

Adds a deterministic offset of up to ±0.009° (approximately ±1 km) to each
coordinate of a geographic position.

**Use for**: latitude/longitude pairs stored as an array or as separate
`lat` / `lng` fields.

**Example**: `[48.8566, 2.3522]` → `[48.8491, 2.3441]`

---

## STATIC_MAPPING

Maps the input value to one of five tokens (`A`, `B`, `C`, `D`, `E`)
deterministically (based on a hash of the input).  The same input always
maps to the same token, so grouping and aggregation queries still work.

**Use for**: low-cardinality categorical fields (status codes, tier names)
where the distribution is more important than the actual values.

**Example**: `"premium"` → `"C"`

---

## STATIC_BLOB_REPLACEMENT

Replaces the entire value with the literal string `[REDACTED]`.

**Use for**: free-text blobs (comments, notes, descriptions) where no
structural information needs to be preserved.

**Example**: `"Great location near the Eiffel Tower!"` → `"[REDACTED]"`
