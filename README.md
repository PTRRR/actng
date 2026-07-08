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
Review transactions that the model is unsure about. You can assign tags, create new ones, or skip.
```bash
actng review ./statements
```

### 4. Tag and Export
Once trained, apply the tags to all discovered entries and export the results.
```bash
actng export ./statements --output final_tagged.csv --summary
```

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

## Configuration

You can specify the profile location using the `ACTNG_PROFILE` environment variable. If not set, it defaults to a standard location in your home directory.
