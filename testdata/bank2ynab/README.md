# Importer test fixtures

Anonymized bank CSV export samples from the [bank2ynab](https://github.com/bank2ynab/bank2ynab)
project (MIT license), copied from `test/test-data/` at commit
`4fa82e9af2c800aa98931a2a0c8b8f1b7ed7c3ac`.

They cover real-world export quirks from banks across Europe, the UK, Asia and
Australia: semicolon/tab delimiters, Windows-125x encodings, metadata preambles
before the header, split debit/credit columns, decimal commas, currency
symbols, and a variety of date formats. Used by
`crates/actng-core/tests/fixtures.rs`.
