import os
from functools import lru_cache

from app_paths import asset_root
from PySide6 import QtGui, QtCore, QtSvg, QtWidgets
from PySide6.QtGui import QPixmap

import darkdetect
colorMode = 'dark' if darkdetect.isDark() else 'light'


@lru_cache(maxsize=128)
def _render_svg(path, size, color_name):
    """Rasterise one SVG asset.

    The bundled icons are drawn in ``currentColor``; substituting it in-memory
    lets one source asset serve both application themes instead of needing a
    ``_light``/``_dark`` pair per icon.

    Cached, because the assets ship with the app and the popup re-creates every
    button icon each time edit mode is toggled.  Keyed on primitives rather
    than a QColor so the cache stays hashable.  The returned QPixmap is
    implicitly shared and callers only ever hand it to setIcon/setPixmap, so
    handing the same instance to several of them is safe.
    """
    try:
        with open(path, "rb") as source:
            svg = source.read()
    except OSError:
        return QPixmap()

    if color_name is not None:
        svg = svg.replace(b"currentColor", color_name.encode("ascii"))

    renderer = QtSvg.QSvgRenderer(svg)
    if not renderer.isValid():
        return QPixmap()

    pixmap = QPixmap(size, size)
    pixmap.fill(QtCore.Qt.GlobalColor.transparent)
    painter = QtGui.QPainter(pixmap)
    renderer.render(painter, _fit(renderer.defaultSize(), size))
    painter.end()
    return pixmap


def _fit(default_size, size):
    """Centre an SVG's natural aspect ratio inside a square ``size`` box.

    Every bundled asset is square today, so this is a no-op for them; it keeps
    a future non-square viewBox from being silently stretched.
    """
    if default_size.width() <= 0 or default_size.height() <= 0:
        return QtCore.QRectF(0, 0, size, size)
    scaled = default_size.scaled(size, size, QtCore.Qt.AspectRatioMode.KeepAspectRatio)
    return QtCore.QRectF(
        (size - scaled.width()) / 2,
        (size - scaled.height()) / 2,
        scaled.width(),
        scaled.height(),
    )


class UIUtils:
    @classmethod
    def svg_pixmap(cls, path, size=96, color=None):
        """Render the SVG at ``path`` to a transparent ``size``-square pixmap,
        optionally recolored.  See :func:`_render_svg`."""
        return _render_svg(str(path), size, color.name() if color is not None else None)

    @classmethod
    def icon_pixmap(cls, icon_name, size=96, color=None):
        """Rasterise ``assets/icons/<icon_name>.svg``.

        ``icon_name`` may carry an ``icons/`` prefix: options files store the
        icon as ``icons/<name>`` and existing user options.json files still do,
        so the prefix is stripped rather than rejected.
        """
        name = str(icon_name).rsplit("/", 1)[-1]
        return cls.svg_pixmap(asset_root() / "icons" / f"{name}.svg", size, color)

    @classmethod
    def theme_color(cls):
        """The foreground color icons are drawn in for the active theme."""
        return QtGui.QColor("white" if colorMode == "dark" else "black")

    @classmethod
    def themed_icon(cls, icon_name, size=96):
        """Return a light- or dark-theme QIcon for an unsuffixed SVG name."""
        return QtGui.QIcon(cls.icon_pixmap(icon_name, size, cls.theme_color()))

    @classmethod
    def provider_logo_pixmap(cls, provider_logo, size=30):
        """Return a provider's SVG logo at its header size.

        Passing the theme color themes the monochrome brand marks, which are
        drawn in ``currentColor`` and would otherwise be black-on-black in dark
        mode.  Marks with their own palette (Gemini's gradient) carry no
        ``currentColor`` token, so the substitution leaves them untouched.
        """
        return cls.icon_pixmap(f"provider_{provider_logo}", size, cls.theme_color())

    @classmethod
    def clear_layout(cls, layout):
        """
        Clear the layout of all widgets.
        """
        while ((child := layout.takeAt(0)) != None):
            #If the child is a layout, delete it
            if child.layout():
                cls.clear_layout(child.layout())
                child.layout().deleteLater()
            else:
                child.widget().deleteLater()

    @classmethod
    def setup_window_and_layout(cls, base: QtWidgets.QWidget):
        # Set the window icon
        icon_path = os.path.join(asset_root(), 'icons', 'app_icon.png')
        if os.path.exists(icon_path): base.setWindowIcon(QtGui.QIcon(icon_path))
        main_layout = QtWidgets.QVBoxLayout(base)
        main_layout.setContentsMargins(0, 0, 0, 0)
        base.background = ThemeBackground(base)
        main_layout.addWidget(base.background)


class ThemeBackground(QtWidgets.QWidget):
    """Paint the application's solid light- or dark-mode background."""

    def __init__(self, parent=None, border_radius=0):
        super().__init__(parent)
        self.setAttribute(QtCore.Qt.WA_StyledBackground, True)
        self.border_radius = border_radius

    def paintEvent(self, event):
        painter = QtGui.QPainter(self)
        painter.setRenderHint(QtGui.QPainter.RenderHint.Antialiasing, True)
        color = QtGui.QColor(35, 35, 35) if colorMode == 'dark' else QtGui.QColor(222, 222, 222)
        painter.setBrush(color)
        painter.setPen(QtCore.Qt.PenStyle.NoPen)
        painter.drawRoundedRect(self.rect(), self.border_radius, self.border_radius)
