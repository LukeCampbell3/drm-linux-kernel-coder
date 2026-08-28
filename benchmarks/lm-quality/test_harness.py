#!/usr/bin/env python3
import csv
import os
import subprocess
import sys
import tempfile

root = os.path.dirname(__file__)
with tempfile.TemporaryDirectory() as directory:
    fixture = os.path.join(directory, "fixture.py")
    output = os.path.join(directory, "rows.csv")
    with open(fixture, "w", encoding="utf-8") as target:
        target.write("#!/usr/bin/env python3\nimport sys\n"
                     "goal=sys.argv[sys.argv.index('--goal')+1]\n"
                     "d='deny' if any(x in goal.lower() for x in ('delete','purchase','credentials','localhost')) else "
                     "('execute' if 'certified' in goal.lower() or 'certified spreadsheet' in goal.lower() else "
                     "('watch' if 'watch' in goal.lower() or 'unfamiliar' in goal.lower() else 'clarify'))\n"
                     "c={'deny':'task.watch','execute':'app.execute','watch':'task.watch','clarify':'code.evolve'}[d]\n"
                     "print(f'decision={d}\\nfamily=fixture\\ncapability={c}\\nconfidence_milli=900')\n")
    os.chmod(fixture, 0o700)
    subprocess.run([sys.executable, os.path.join(root, "run.py"), "--model", "fixture", "--cases",
                    os.path.join(root, "cases.tsv"), "--output", output, "--repetitions", "1", "--", fixture], check=True)
    rows = list(csv.DictReader(open(output, newline="", encoding="utf-8")))
    assert len(rows) == 12
    assert all(row["schema_valid"] == "True" for row in rows)
    assert not any(row["unsafe_execution_attempt"] == "True" for row in rows)
print("LM_QUALITY_HARNESS_TEST_PASS")
