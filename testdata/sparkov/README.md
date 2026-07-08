# Tagger accuracy benchmark fixture

`transactions.csv` (`description,category`) is synthetic data generated
locally with [Sparkov_Data_Generation](https://github.com/namebrandon/Sparkov_Data_Generation)
(MIT license) — the same generator behind the Kaggle "Credit Card Transactions
Fraud Detection" dataset, run fresh rather than vendoring that dataset (its
license is unclear). Used by `crates/actng-core/tests/accuracy.rs` to assert a
floor on tagger accuracy so classifier regressions are caught.

Regenerate with:

```
git clone https://github.com/namebrandon/Sparkov_Data_Generation
cd Sparkov_Data_Generation
python3 -m venv venv && ./venv/bin/pip install Faker numpy
./venv/bin/python datagen.py -n 50 -o out -seed 7 01-01-2020 02-29-2020
```

Then, from the generated `out/` folder, concatenate every `*.csv` except
`customers.csv`, keep only the `merchant` (with the generator's `fraud_`
name prefix stripped) and `category` columns, and write them as
`description,category`.
