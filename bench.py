#!/usr/bin/env python3
import json
from pathlib import Path
import time
import automerge

edits = json.loads(Path("edits.json").read_text())

commands = []

for i in range(len(edits)):
    pos = edits[i][0]
    d = edits[i][1]

    vals = ""
    for j in range(2, len(edits[i])):
        vals += edits[i][j]

    commands.append((pos, d, vals));


for _ in range(100):
    print(_)
    doc = automerge.init()
    now = time.perf_counter()
    with automerge.transaction(doc) as d:
        d.text = automerge.Text("")
        t = d.text
        for (pos, dd, vals) in commands:
            t[pos:pos+dd] = vals


    print(f"Done in {time.perf_counter() - now} s", );

# print(doc.text)
# let save = Instant::now();
# let bytes = doc.save();
# println!("Saved in {} ms", save.elapsed().as_millis());

# let load = Instant::now();
# let _ = Automerge::load(&bytes).unwrap();
# println!("Loaded in {} ms", load.elapsed().as_millis());

# let get_txt = Instant::now();
# doc.text(&text)?;
# println!("Text in {} ms", get_txt.elapsed().as_millis());

# Ok(())
