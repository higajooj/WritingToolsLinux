"""Pin Qt to the offscreen platform for the test suite.

Import this before PySide6.  It *sets* QT_QPA_PLATFORM rather than defaulting
it: a desktop session exports the variable itself (Hyprland and KDE both set
``wayland;xcb``), which makes ``os.environ.setdefault`` a silent no-op and
leaves the suite depending on a live display it should never need.  Export
WRITING_TOOLS_TEST_QPA to run against a real platform plugin instead.
"""

import os

os.environ["QT_QPA_PLATFORM"] = os.environ.get("WRITING_TOOLS_TEST_QPA", "offscreen")
# The GTK platform theme plugin needs a display and only warns without one.
os.environ.pop("QT_QPA_PLATFORMTHEME", None)
