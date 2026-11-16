# Change-password pages

`urls.tsv` is Apple's published list of sites' change-password pages, one
site per line: the domain, a tab, and the page's address. It's
`quirks/change-password-URLs.json` from
https://github.com/apple/password-manager-resources at commit
2f72ab38f8b252ad0b7698e4440efb9a2710aa9d, copyright Apple Inc., under the MIT
License (`LICENSE-APPLE.md`). It's the same list, at the same commit, that the
web vault and the browser extension use (passbundle-clients,
`libs/common/src/tools/keykeeper/site-change-password-urls.data.ts`, made by
`scripts/keykeeper/update-site-data.mjs`), written out as tab-separated lines
from that file. The entries are unchanged.
