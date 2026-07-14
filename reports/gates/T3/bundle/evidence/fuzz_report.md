# Tier 3 Local Fuzz Report

Reviewed product: `7bbbafa376a5222c3a335a744b5b942898c67a84`

Reviewed tree: `270d978f32921a29e92492fc2a782bf60bab0bc2`

Metric SHA-256: `1c45919f6f8ebaf719065d6e78da324a80a3f365a141e1417a95850b012a63ef`

## Result

PASS. Eight local AddressSanitizer workers completed 3,608 verified worker-seconds and 43,243,299 executions.

Required targets: fuzz_apply, fuzz_carddb, fuzz_carddsl, fuzz_characteristics, fuzz_scenarioparse.

Every worker returned zero, emitted final libFuzzer statistics, met its requested duration, and produced no crash artifact. Full logs are archived under `reports/gates/T3/fuzz/` and hash-bound by `metrics/local_fuzz.json`.

The campaign was local and offline. It used no GitHub Actions, network access, install, push, or PR.
