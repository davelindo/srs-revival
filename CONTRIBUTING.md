# Contributing

Useful contributions include verified device observations, corrections,
independent reproduction of the X7 procedure, and original source tools.
Include the model, board revision if known, firmware version and relevant
input/output hashes with device reports.

Write owner documentation as a reference. Describe what the feature does,
the firmware or connection it requires, and any limits an owner needs to know.
[Owner guide](docs/roadmap.md)

Do not attach firmware, patched images, Sony manuals, apps, extracted
resources, decompiled source or full device logs/backups to issues, PRs,
releases or Pages. Provide original explanations and public vendor links.
Keep Wi-Fi secrets, serial/MAC addresses, tokens and unit-specific trust or
calibration data private. Register values and SPI bodies can be sensitive too.

## Build the documentation

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
.venv/bin/python -m unittest discover -s tests
.venv/bin/python tools/build_site.py
.venv/bin/python tools/check_publication.py --site _site
.venv/bin/python -m http.server 8765 --bind 127.0.0.1 --directory _site
```

Original contributions use the repository's MIT license. Preserve applicable
notices for any open-source code reused from elsewhere.
