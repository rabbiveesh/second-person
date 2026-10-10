# Adaptive difficulty: tuning notes

The rules are in the module doc (`src/adapt/mod.rs`); what each band and the dial do to the game
is in `src/difficulty.rs`. This file records how the engine's constants were picked, using the
simulator:

```sh
cargo run --example simulate -- --all --seeds 20            # summary, 20 seeds averaged
cargo run --example simulate -- --profile brawler --seed 3  # round by round
```

The simulator plays the real reducer and `next_round`. Each synthetic player has a logistic
success curve per skill and band. Stealth decides whether they land the first hit before he
engages; if he engages anyway (70% of the time after a sneaky hit), Gunfight decides the fight,
with an edge for having hit first. Assists remove `ASSIST_HELP` = 50% of the remaining failure
chance at a full dial. Metrics: **wins** = win rate; **warm** = after the first 30 rounds;
**near** = share of rounds whose trailing-20 win rate is within 60–85%; **osc** = band changes
that reverse the same skill's previous change within 25 rounds (ping-pong, both skills summed);
**rev** = all reversals; **dial** = mean assist dial; **maxed** = rounds with the dial ≥ 0.9;
**unast** = rounds played fully unassisted (no sonar in `Auto`, minimal magnetism).

## Result (200 rounds, seeds 0..19)

| profile   | wins | warm | near |  osc |  rev |  prom/dem | frust | dial | maxed | unast | centers |
|-----------|------|------|------|------|------|-----------|-------|------|-------|-------|---------|
| precise   |  80% |  80% |  76% |  4.4 |  7.1 |  6.4/3.5  |   2.1 | 0.04 |    0% |   71% | 7.2 7.3 |
| sloppy    |  79% |  79% |  81% |  4.0 |  6.4 |  7.4/3.8  |   3.8 | 0.04 |    0% |   69% | 4.2 4.4 |
| beginner  |  64% |  65% |  75% |  0.5 |  0.5 |  0.6/1.4  |  12.1 | 0.14 |    0% |   33% | 1.0 1.0 |
| brawler   |  80% |  80% |  74% |  2.5 |  4.6 |  4.7/6.8  |   3.7 | 0.04 |    0% |   71% | 1.0 7.4 |
| sneak     |  76% |  76% |  82% |  2.4 |  4.1 |  6.8/3.2  |   3.1 | 0.05 |    0% |   64% | 7.1 1.9 |
| learner   |  82% |  83% |  66% |  3.8 |  6.4 | 10.1/3.7  |   1.9 | 0.03 |    0% |   77% | 4.8 5.2 |

## What changed while tuning, and why

1. **The starting point was the platformer's rules as written** (4 rounds of evidence, 8 to
   re-promote). There a room exercises one skill out of nine, so each skill sees a room every
   few minutes. Here every round is evidence for both skills, so evidence arrives ~5× denser and
   the same thresholds ping-ponged: precise oscillated 10.6 times per 200 rounds.
2. **Evidence 5, re-promote 10.** Fewer ping-pongs without letting players drift easy. Going
   to 8/16 cut oscillation to ~2 but pushed win rates to 85–89% (above target) and stranded
   players two bands under their level.
3. **Re-promotion needs stretch evidence.** A player whose true level sits between bands
   genuinely clears ≥75% at the band below, so more center evidence alone never stops
   7→8→7→8. Going back into a band just fallen out of now also needs ≥2 stretch rounds at ≥60%:
   show it at the harder band first. Cut precise's oscillation from 6.2 to 4.4 at 5/10.
4. **One quick loss isn't frustration.** Counting every loss under 20s eased a band each time a
   player charged in, and dragged sloppy two bands under its level. A quick loss now has to
   follow another loss.

## Known limits

- **A skill nobody uses sinks to band 1.** `brawler` never sneaks, so Stealth falls to 1: he
  becomes half-blind and hard of hearing for a player who walks straight at him. That's
  arguably right (they still win 80% by fighting), but watch it in playtests; a floor on
  Stealth for players with a high Gunfight center is the obvious lever.
- **Fast learners outpace the evidence** (`learner`: 83% after warm-up, 66% near target). It
  errs on the easy side.
- **beginner** sits at the floor (band 1, dial ~0.14) and still wins 64%. Below that the
  game has nothing gentler to offer short of a bigger dial range.
- Every curve in `difficulty.rs` is set by feel around the hand-tuned game at band 5 and the
  baseline dial. Re-tune once there's real playtest data.
