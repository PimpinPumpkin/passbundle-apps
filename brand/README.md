# Brand

What the agent and the app show and trust as PassBundle, baked in at build time:

- `brand.json`: the name, domain, web vault address, and native-messaging host name. The same file as `brand/brand.json` in passbundle-clients, which brands the web vault and the extension; tier 1 (in the meta repo) checks the two agree.
- `agent/`: the agent's tray icons, as passbundle-clients' `brand/render-assets.sh` draws them.
- `mark.svg`: the mark, the same as passbundle-clients' `brand/assets/mark.svg`. `render-app-icons.sh` draws the app's icon from it, the mark in white on the logo's emerald jewel tile (as passbundle-clients' `brand/wordmark.py` draws the tile), for macOS, iOS, Windows, Linux, and Android, into `app/`. The app draws the full logo, the tile and the two-tone "passbundle", itself (`app/lib/widgets.dart`, `Wordmark`).
