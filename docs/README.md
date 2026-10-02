# Board boot guides

`boot-guide.json` is the shared source for the instructions embedded in TOBI and the static website. It covers every board in the root catalog, including boards without onboard eMMC. Keep switch values in **printed switch-number order**, and record the corresponding BOOTMODE signal, source manual, and review date. Diagrams are original SVG illustrations generated from the same values; they are not board photographs.

Build and preview with Python 3; no third-party packages are required:

```sh
python3 docs/build.py
python3 -m http.server 8080 --directory out/docs-site
```

Open `http://localhost:8080`. Production board URLs use `https://texasinstruments.github.io/TOBI/boards/<board-id>/`, matching the QR codes shown after a successful eMMC installation.

To publish, a repository administrator must enable **Settings → Pages → Build and deployment → Source: GitHub Actions**. The `Publish board boot guides` workflow always builds and uploads the site artifact, and deploys changes from `master` once that Pages configuration is enabled. If setup is missing, its summary explains how to enable hosting. Rerun the workflow on `master` after enabling Pages. Its manual trigger also deploys only `master`; developing guides on `dev` does not change the public website. See [GitHub's custom-workflow documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).

TOBI displays the instructions locally even when the network is unavailable. The QR destination needs network access and a published Pages site. Rebuild the application and board images after changing the embedded instructions. The guides distinguish filesystem boot from hardware Boot0 boot and explain any supported bootstrap. Do not copy settings from a similarly named EVM or claim success for an unprepared bootloader layout.

The `v2026.10.2-r2` board images include these shared guides and automatic SK-AM69 Boot0 preparation. The original `v2026.10.2` release predates those changes, despite sharing the app's date version.
