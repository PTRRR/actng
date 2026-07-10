# actng

`actng` is a portable, learned tagging system for bank statement imports. It helps you categorize your transactions using a combination of exact-match memory and a Naive Bayes classifier, allowing you to "train" your tagging rules over time without writing complex regex patterns.

## Features

- **Learned Tagging**: Teach the tool by reviewing transactions; it learns patterns automatically.
- **Portable Profiles**: All your tags and training data are stored in a single JSON profile.
- **Smart Discovery**: Automatically finds and parses bank CSVs in your directories.
- **Deduplication**: Prevents duplicate entries when importing overlapping statement files.
- **Flexible Exports**: Export your tagged data to CSV for use in other accounting software.

## Quick Start

### 1. Initialize your profile
Create a new profile to start storing your tags and training data.
```bash
actng init
```

### 2. Scan your statements
Point `actng` to the directory containing your bank exports to see what it finds.
```bash
actng scan ./statements
```

### 3. Train the model
Review transactions that the model is unsure about using the interactive TUI.
```bash
actng review ./statements
```

#### TUI Hotkeys
- `1-9`: Confirm the corresponding suggestion.
- `t` or `n`: Manually assign a tag (or create a new one).
- `x`: Tag as an exception — pins the tag to this one entry without training the classifier (see [Exceptions](#exceptions)).
- `s`: Skip the current transaction.
- `u`: Undo the last training step.
- `a`: Toggle "Review All" mode (include confidently tagged entries).
- `q`: Quit.

On the Entries screen, `x` retags the selected entry as an exception the same way; `Enter` retags it normally (and clears any exception the entry had). `f` cycles through `all`/`tagged`/`review`/`overridden`/per-tag filters.


### 4. Tag and Export
Once trained, apply the tags to all discovered entries and export the results.
```bash
actng export ./statements --output final_tagged.csv --summary
```

## Exceptions

Training keys off the normalized description, so two purchases at the same
merchant always get the same tag — that's the point, but it means one-off
purchases (a `gift` bought at your usual `groceries` shop) can't be
corrected without also flipping every other entry at that merchant. An
**exception** (or override) pins a tag to one concrete entry — matched on
its exact date, description and amount — without training the classifier
at all. It survives re-imports and re-runs like everything else in the
profile, but it never cascades to other entries and never touches the
Bayes counts.

Create one from `actng review` (choose "tag as exception…") or from the TUI
with `x` on the Review or Entries screen. Retagging an entry normally
(`Enter` in the TUI, or a normal review answer) clears any exception it had
first, so the new decision isn't silently shadowed.

## CLI Reference

### Profile Management
- `actng init`: Initialize a new profile.
- `actng profile-info`: Show statistics about your tags and training data.

### Data Processing
- `actng scan <dir>`: List discovered files and entry counts.
- `actng tag <dir>`: Preview how the current profile would tag the entries.
- `actng review <dir>`: Interactive session to train the classifier.
- `actng export <dir> --output <file> [--summary]`: Export tagged entries to CSV.

### Tag Management
`actng tags` allows you to manage your taxonomy:
- `actng tags add <tag>`: Add a new tag.
- `actng tags rm <tag>`: Remove a tag and its associated training data.
- `actng tags category <tag> <category>`: Assign a tag to a category (e.g., "Netflix" $\rightarrow$ "Entertainment").
- `actng tags list`: List all tags and their categories.

### Exceptions
`actng overrides` manages per-entry tag exceptions (see [Exceptions](#exceptions)):
- `actng overrides list`: List every exception with its index, date, amount, description and tag.
- `actng overrides rm <index>`: Remove an exception by the index `list` printed.

## Configuration

You can specify the profile location using the `ACTNG_PROFILE` environment variable. If not set, it defaults to a standard location in your home directory.
