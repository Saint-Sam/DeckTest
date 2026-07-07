# T1 Replay Corpus

Generated on 2026-07-07 with:

```bash
cargo run -q -p forge-cli -- play --demo --seed <seed> --replay-out reports/gates/T1/replays/demo-seed-<seed>.frsreplay
cargo run -q -p forge-cli -- roundtrip reports/gates/T1/replays/demo-seed-<seed>.frsreplay
```

Seeds `11` through `20` all round-tripped successfully.

| Seed | Final hash | Outcome |
| --- | --- | --- |
| 11 | 17913199206715572167 | won player 0 |
| 12 | 18224083063841829252 | won player 0 |
| 13 | 6378427721804725294 | won player 0 |
| 14 | 11845241288509108833 | won player 0 |
| 15 | 14380285740381315058 | won player 0 |
| 16 | 5890165003627006339 | won player 0 |
| 17 | 83314587503209014 | won player 0 |
| 18 | 13073541269431405533 | won player 0 |
| 19 | 13708373468767253346 | won player 0 |
| 20 | 713360490662108421 | won player 0 |
