# Soundfont

`gm.sf2` is [GeneralUser GS](http://www.schristiancollins.com) v1.471 by S. Christian Collins
(license in `GeneralUser-GS-LICENSE.txt`), cut down to the presets the tracks play:

```
cargo run --release --example trim_soundfont -- GeneralUser.sf2 assets/music/gm.sf2 \
  0:0 0:11 0:32 0:56 0:57 0:66 128:0=62,63,64,65,66,75,76
```

`bank:program` keeps a preset; `128:0=keys` keeps only the drum kit's zones for those GM keys.
A track that wants another instrument or drum adds it here and re-runs the trim.
