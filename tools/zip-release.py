"""Packs a Windows release's folder as the release's zip: everything in
<folder>/PassBundle under PassBundle/, folders too (so an empty one comes
back, as release.json lists it), with forward slashes (as the zip format has
them, whatever Windows says), deflated, in a fixed order.

  python tools/zip-release.py <folder> <zip>

The Windows release workflow (windows-release.yml) and
tools/desktop-update-test.sh both pack with this.
"""

import os
import sys
import zipfile

folder, out = sys.argv[1], sys.argv[2]
top = os.path.join(folder, "PassBundle")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for root, dirs, files in os.walk(top):
        dirs.sort()
        z.write(root, os.path.relpath(root, folder).replace(os.sep, "/") + "/")
        for name in sorted(files):
            path = os.path.join(root, name)
            z.write(path, os.path.relpath(path, folder).replace(os.sep, "/"))
