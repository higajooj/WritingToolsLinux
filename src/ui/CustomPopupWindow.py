import logging
import sys
from functools import partial

from PySide6 import QtCore, QtGui, QtWidgets
from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QPushButton,
    QRadioButton,
    QVBoxLayout,
    QWidget,
)

from options_store import (
    load_options as load_options_file,
    reset_options as reset_options_file,
    save_options as save_options_file,
)
from platform_input import validate_trigger
from ui.UIUtils import ThemeBackground, UIUtils, colorMode

_ = lambda x: x

class ButtonEditDialog(QDialog):
    """
    Dialog for editing or creating a button's properties
    (name/title, system instruction, open_in_window, etc.).
    """
    def __init__(self, parent=None, button_data=None, title="Edit Button"):
        super().__init__(parent)
        self.button_data = button_data if button_data else {
            "prefix": "Make this change to the following text:\n\n",
            "instruction": "",
            "icon": "icons/magnifying-glass",
            "open_in_window": False
        }
        # The hotkey input is created in init_ui; tracked here so the
        # parent window can read it back from get_button_data().
        self.hotkey_input = None
        self.setWindowTitle(title)
        self.init_ui()
        
    def init_ui(self):
        layout = QVBoxLayout(self)
        
        # Name
        name_label = QLabel("Button Name:")
        name_label.setStyleSheet(f"color: {'#fff' if colorMode == 'dark' else '#333'}; font-weight: bold;")
        self.name_input = QLineEdit()
        self.name_input.setStyleSheet(f"""
            QLineEdit {{
                padding: 8px;
                border: 1px solid {'#777' if colorMode == 'dark' else '#ccc'};
                border-radius: 8px;
                background-color: {'#333' if colorMode == 'dark' else 'white'};
                color: {'#fff' if colorMode == 'dark' else '#000'};
            }}
        """)
        if "name" in self.button_data:
            self.name_input.setText(self.button_data["name"])
        layout.addWidget(name_label)
        layout.addWidget(self.name_input)
        
        # Instruction (changed to a multiline QPlainTextEdit)
        instruction_label = QLabel("What should your AI do with your selected text? (System Instruction)")
        instruction_label.setStyleSheet(f"color: {'#fff' if colorMode == 'dark' else '#333'}; font-weight: bold;")
        self.instruction_input = QPlainTextEdit()
        self.instruction_input.setStyleSheet(f"""
            QPlainTextEdit {{
                padding: 8px;
                border: 1px solid {'#777' if colorMode == 'dark' else '#ccc'};
                border-radius: 8px;
                background-color: {'#333' if colorMode == 'dark' else 'white'};
                color: {'#fff' if colorMode == 'dark' else '#000'};
            }}
        """)
        self.instruction_input.setPlainText(self.button_data.get("instruction", ""))
        self.instruction_input.setMinimumHeight(100)
        self.instruction_input.setPlaceholderText("""Examples:
    - Fix / improve / explain this code.
    - Make it funny.
    - Add emojis!
    - Roast this!
    - Translate to English.
    - Make the text title case.
    - If it's all caps, make it all small, and vice-versa.
    - Write a reply to this.
    - Analyse potential biases in this news article.""")
        layout.addWidget(instruction_label)
        layout.addWidget(self.instruction_input)
        
        # open_in_window
        display_label = QLabel("How should your AI response be shown?")
        display_label.setStyleSheet(f"color: {'#fff' if colorMode == 'dark' else '#333'}; font-weight: bold;")
        layout.addWidget(display_label)
        
        radio_layout = QHBoxLayout()
        self.clipboard_radio = QRadioButton("Copy to clipboard")
        self.window_radio = QRadioButton("In a pop-up window (with follow-up support)")
        for r in (self.clipboard_radio, self.window_radio):
            r.setStyleSheet(f"color: {'#fff' if colorMode == 'dark' else '#333'};")
        
        self.clipboard_radio.setChecked(not self.button_data.get("open_in_window", False))
        self.window_radio.setChecked(self.button_data.get("open_in_window", False))

        radio_layout.addWidget(self.clipboard_radio)
        radio_layout.addWidget(self.window_radio)
        layout.addLayout(radio_layout)

        # Direct hotkey (optional). Lets the user fire this button from
        # anywhere without opening the popup first. Stored per-button in
        # options.json under a "hotkey" key; absent = no hotkey, which is
        # how every existing/legacy button starts.
        hotkey_label = QLabel("Direct hotkey (optional):")
        hotkey_label.setStyleSheet(f"color: {'#fff' if colorMode == 'dark' else '#333'}; font-weight: bold;")
        layout.addWidget(hotkey_label)

        self.hotkey_input = QLineEdit()
        self.hotkey_input.setStyleSheet(f"""
            QLineEdit {{
                padding: 8px;
                border: 1px solid {'#777' if colorMode == 'dark' else '#ccc'};
                border-radius: 8px;
                background-color: {'#333' if colorMode == 'dark' else 'white'};
                color: {'#fff' if colorMode == 'dark' else '#000'};
            }}
        """)
        self.hotkey_input.setPlaceholderText("e.g. ctrl+j  (leave blank for none)")
        self.hotkey_input.setText(self.button_data.get("hotkey", ""))
        layout.addWidget(self.hotkey_input)

        hotkey_hint = QLabel(
            "Press this combination from anywhere to run this button "
            "directly, skipping the popup.\nUse '+' between keys, e.g. "
            "ctrl+j or ctrl+shift+p."
        )
        hotkey_hint.setStyleSheet(
            f"color: {'#bbb' if colorMode == 'dark' else '#555'}; font-size: 12px;"
        )
        hotkey_hint.setWordWrap(True)
        layout.addWidget(hotkey_hint)

        # OK & Cancel
        btn_layout = QHBoxLayout()
        ok_button = QPushButton("OK")
        cancel_button = QPushButton("Cancel")
        for btn in (ok_button, cancel_button):
            btn.setStyleSheet(f"""
                QPushButton {{
                    background-color: {'#444' if colorMode == 'dark' else '#f0f0f0'};
                    color: {'#fff' if colorMode == 'dark' else '#000'};
                    border: 1px solid {'#666' if colorMode == 'dark' else '#ccc'};
                    border-radius: 5px;
                    padding: 8px;
                    min-width: 100px;
                }}
                QPushButton:hover {{
                    background-color: {'#555' if colorMode == 'dark' else '#e0e0e0'};
                }}
            """)
        btn_layout.addWidget(ok_button)
        btn_layout.addWidget(cancel_button)
        layout.addLayout(btn_layout)
        
        ok_button.clicked.connect(self.accept)
        cancel_button.clicked.connect(self.reject)
        
        self.setStyleSheet(f"""
            QDialog {{
                background-color: {'#222' if colorMode == 'dark' else '#f5f5f5'};
                border-radius: 10px;
            }}
        """)

    def get_button_data(self):
        data = {
            "name": self.name_input.text().strip(),
            "prefix": "Make this change to the following text:\n\n",
            # Retrieve multiline text
            "instruction": self.instruction_input.toPlainText(),
            "icon": "icons/custom",
            "open_in_window": self.window_radio.isChecked()
        }
        # Only include `hotkey` if the user actually typed one. Old
        # configs and buttons-without-hotkeys stay shaped exactly as
        # before — no empty-string clutter in options.json.
        hotkey = self.hotkey_input.text().strip().lower() if self.hotkey_input else ""
        if hotkey:
            data["hotkey"] = hotkey
        return data

class DraggableButton(QtWidgets.QPushButton):
    def __init__(self, parent_popup, key, text):
        super().__init__(text, parent_popup)
        self.popup = parent_popup
        self.key = key
        self.drag_start_position = None
        self.setAcceptDrops(True)
        self.icon_container = None

        # Enable mouse tracking and hover events, and styled background
        self.setMouseTracking(True)
        self.setAttribute(QtCore.Qt.WA_Hover, True)
        self.setAttribute(QtCore.Qt.WA_StyledBackground, True)

        # Use a dynamic property "hover" (default False)
        self.setProperty("hover", False)
        self.setProperty("selected", False)

        # Set fixed size (adjust as needed)
        self.setFixedSize(120, 40)

        # Define base style using the dynamic property instead of the :hover pseudo-class
        self.base_style = f"""
            QPushButton {{
                background-color: {"#444" if colorMode=="dark" else "white"};
                border: 1px solid {"#666" if colorMode=="dark" else "#ccc"};
                border-radius: 8px;
                padding: 10px;
                font-size: 14px;
                text-align: left;
                color: {"#fff" if colorMode=="dark" else "#000"};
            }}
            QPushButton[hover="true"] {{
                background-color: {"#555" if colorMode=="dark" else "#f0f0f0"};
            }}
            QPushButton[selected="true"] {{
                background-color: {"#315b36" if colorMode=="dark" else "#e5f4e7"};
                border-color: {"#66bb6a" if colorMode=="dark" else "#4CAF50"};
            }}
        """
        self.setStyleSheet(self.base_style)
        logging.debug("DraggableButton initialized")

    def enterEvent(self, event):
        # Only update the hover property if NOT in edit mode.
        if not self.popup.edit_mode:
            self.setProperty("hover", True)
            self.style().unpolish(self)
            self.style().polish(self)
        super().enterEvent(event)

    def leaveEvent(self, event):
        if not self.popup.edit_mode:
            self.setProperty("hover", False)
            self.style().unpolish(self)
            self.style().polish(self)
        super().leaveEvent(event)

    def mousePressEvent(self, event):
        if event.button() == QtCore.Qt.LeftButton:
            if self.popup.edit_mode:
                self.drag_start_position = event.pos()
                event.accept()
                return
        super().mousePressEvent(event)
            
    def mouseMoveEvent(self, event):
        if not (event.buttons() & QtCore.Qt.LeftButton) or not self.drag_start_position:
            return

        distance = (event.pos() - self.drag_start_position).manhattanLength()
        if distance < QtWidgets.QApplication.startDragDistance():
            return

        if self.popup.edit_mode:
            drag = QtGui.QDrag(self)
            mime_data = QtCore.QMimeData()
            idx = self.popup.button_widgets.index(self)
            mime_data.setData("application/x-button-index", str(idx).encode())
            drag.setMimeData(mime_data)

            pixmap = self.grab()
            drag.setPixmap(pixmap)
            drag.setHotSpot(event.pos())

            self.drag_start_position = None
            try:
                drop_action = drag.exec_(QtCore.Qt.MoveAction)
            finally:
                # A cancelled drag has no drop event to remove the indicator.
                self.popup.clear_drop_indicator()
            logging.debug(f"Drag completed with action: {drop_action}")

    def _drop_side(self, event):
        """Return which edge of this button represents the insertion gap."""
        return "before" if event.position().x() < self.width() / 2 else "after"

    def _update_drop_indicator(self, event):
        self.popup.show_drop_indicator(self, self._drop_side(event))

    def dragEnterEvent(self, event):
        if self.popup.edit_mode and event.mimeData().hasFormat("application/x-button-index"):
            self._update_drop_indicator(event)
            event.acceptProposedAction()
        else:
            event.ignore()

    def dragMoveEvent(self, event):
        if self.popup.edit_mode and event.mimeData().hasFormat("application/x-button-index"):
            self._update_drop_indicator(event)
            event.acceptProposedAction()
        else:
            event.ignore()

    def dragLeaveEvent(self, event):
        self.popup.clear_drop_indicator(self)
        event.accept()

    def dropEvent(self, event):
        if not self.popup.edit_mode or not event.mimeData().hasFormat("application/x-button-index"):
            event.ignore()
            return

        source_idx = int(event.mimeData().data("application/x-button-index").data().decode())
        bw = self.popup.button_widgets
        # The payload is only as fresh as the moment the drag started, so the
        # buttons it indexed may already be gone. Drop it rather than moving
        # whichever button happens to sit at that index now.
        if not 0 <= source_idx < len(bw):
            logging.debug(f"Ignoring stale drag payload with index {source_idx}")
            self.popup.clear_drop_indicator()
            event.ignore()
            return

        target_idx = self.popup.button_widgets.index(self)
        insert_after = self._drop_side(event) == "after"
        insertion_idx = target_idx + (1 if insert_after else 0)
        original_order = list(self.popup.button_widgets)

        dragged_button = bw.pop(source_idx)
        if source_idx < insertion_idx:
            insertion_idx -= 1
        bw.insert(insertion_idx, dragged_button)

        self.popup.clear_drop_indicator()
        if bw != original_order:
            self.popup.rebuild_grid_layout()
            if not self.popup.update_json_from_grid():
                # Persistence failed, so put the visible order back too.
                bw[:] = original_order
                self.popup.rebuild_grid_layout()

        event.setDropAction(QtCore.Qt.MoveAction)
        event.acceptProposedAction()

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self.icon_container:
            self.icon_container.setGeometry(0, 0, self.width(), self.height())

class CustomPopupWindow(QtWidgets.QWidget):
    def __init__(self, app):
        super().__init__()
        self.app = app
        self.edit_mode = False
        self.selected_option = None

        self.drag_label = None
        self.edit_button = None
        self.reset_button = None
        self.close_button = None
        self.custom_input = None
        self.send_button = None
        self.input_area = None
        
        self.button_widgets = []
        self.drop_indicator_button = None
        self.drop_indicator_side = None

        logging.debug('Initializing CustomPopupWindow')
        self.init_ui()

    def init_ui(self):
        logging.debug('Setting up CustomPopupWindow UI')
        # Wayland needs a compositor rule to float this top-level window.
        self.setWindowFlags(QtCore.Qt.Tool | QtCore.Qt.WindowStaysOnTopHint | QtCore.Qt.FramelessWindowHint)
        self.setAttribute(QtCore.Qt.WA_TranslucentBackground)
        self.setWindowTitle("Writing Tools")
        
        main_layout = QtWidgets.QVBoxLayout(self)
        main_layout.setContentsMargins(0,0,0,0)
        
        self.background = ThemeBackground(
            self, 
            self.app.config.get('theme','gradient'),
            is_popup=True,
            border_radius=10
        )
        main_layout.addWidget(self.background)
        
        content_layout = QtWidgets.QVBoxLayout(self.background)
        # Margin Control
        content_layout.setContentsMargins(10, 4, 10, 10)
        content_layout.setSpacing(10)
        
        # TOP BAR LAYOUT & STYLE
        top_bar = QHBoxLayout()
        top_bar.setContentsMargins(0, 0, 0, 0)
        top_bar.setSpacing(0)

        # The "Edit"/"Done" button (left), same exact size as close button
        self.edit_button = QPushButton()
        self.edit_button.setIcon(UIUtils.themed_icon('pencil'))
        # Reduced size to 24x24 to shrink top bar
        self.edit_button.setFixedSize(24, 24)
        self.edit_button.setStyleSheet(f"""
            QPushButton {{
                background-color: transparent;
                border: none;
                border-radius: 6px;
                padding: 0px;
                margin-top: 3px;
            }}
            QPushButton:hover {{
                background-color: {'#333' if colorMode=='dark' else '#ebebeb'};
            }}
        """)
        self.edit_button.clicked.connect(self.toggle_edit_mode)
        top_bar.addWidget(self.edit_button, 0, Qt.AlignLeft)

        # The label "Drag to rearrange" (BOLD as requested)
        self.drag_label = QLabel("Drag to rearrange")
        self.drag_label.setStyleSheet(f"""
            color: {'#fff' if colorMode=='dark' else '#333'};
            font-size: 14px;
            font-weight: bold; /* <--- BOLD TEXT */
        """)
        self.drag_label.setAlignment(Qt.AlignCenter)
        self.drag_label.hide()
        top_bar.addWidget(self.drag_label, 1, Qt.AlignVCenter | Qt.AlignHCenter)

        # The "Reset" button (edit-mode only) - also 24x24
        self.reset_button = QPushButton()
        self.reset_button.setIcon(UIUtils.themed_icon('restore'))
        self.reset_button.setText("")
        self.reset_button.setFixedSize(24, 24)
        self.reset_button.setStyleSheet(f"""
            QPushButton {{
                background-color: transparent;
                border: none;
                border-radius: 6px;
                padding: 0px;
            }}
            QPushButton:hover {{
                background-color: {'#333' if colorMode=='dark' else '#ebebeb'};
            }}
        """)
        self.reset_button.clicked.connect(self.on_reset_clicked)
        self.reset_button.hide()
        top_bar.addWidget(self.reset_button, 0, Qt.AlignRight)

        # Close button block:
        self.close_button = QPushButton("×")
        self.close_button.setFixedSize(24, 24)
        self.close_button.setStyleSheet(f"""
            QPushButton {{
                background-color: transparent;
                color: {'#fff' if colorMode=='dark' else '#333'};
                font-size: 20px;   /* bigger text */
                font-weight: bold; /* bold text */
                border: none;
                border-radius: 6px;
                padding: 0px;
            }}
            QPushButton:hover {{
                background-color: {'#333' if colorMode=='dark' else '#ebebeb'};
            }}
        """)
        self.close_button.clicked.connect(self.close)
        top_bar.addWidget(self.close_button, 0, Qt.AlignRight)
        content_layout.addLayout(top_bar)

        
        # Input area (hidden in edit mode)
        self.input_area = QWidget()
        input_layout = QHBoxLayout(self.input_area)
        input_layout.setContentsMargins(0,0,0,0)
        
        self.custom_input = QLineEdit()
        self.custom_input.setPlaceholderText(_("Describe your change..."))
        self.custom_input.setStyleSheet(f"""
            QLineEdit {{
                padding: 8px;
                border: 1px solid {'#777' if colorMode=='dark' else '#ccc'};
                border-radius: 8px;
                background-color: {'#333' if colorMode=='dark' else 'white'};
                color: {'#fff' if colorMode=='dark' else '#000'};
            }}
        """)
        self.custom_input.returnPressed.connect(self.on_custom_change)
        input_layout.addWidget(self.custom_input)
        
        self.send_button = QPushButton()
        self.send_button.setIcon(UIUtils.themed_icon('send'))
        self.send_button.setStyleSheet(f"""
            QPushButton {{
                background-color: {'#2e7d32' if colorMode=='dark' else '#4CAF50'};
                border: none;
                border-radius: 8px;
                padding: 5px;
            }}
            QPushButton:hover {{
                background-color: {'#1b5e20' if colorMode=='dark' else '#45a049'};
            }}
        """)
        self.send_button.setFixedSize(self.custom_input.sizeHint().height(),
                                      self.custom_input.sizeHint().height())
        self.send_button.clicked.connect(self.on_custom_change)
        input_layout.addWidget(self.send_button)
        
        content_layout.addWidget(self.input_area)

        self.build_buttons_list()
        self.rebuild_grid_layout(content_layout)

        logging.debug('CustomPopupWindow UI setup complete')
        QtCore.QTimer.singleShot(250, lambda: self.custom_input.setFocus())

    @staticmethod
    def load_options():
        data = load_options_file()
        logging.debug('Options loaded successfully')
        return data

    @staticmethod
    def save_options(options):
        save_options_file(options)

    def build_buttons_list(self, data=None):
        """
        Creates a DraggableButton for each option (except "Custom") from
        `data`, or options.json when omitted, storing them in
        self.button_widgets in the same order.
        """
        self.clear_drop_indicator()
        if data is None:
            data = self.load_options()

        new_widgets = []
        try:
            for k,v in data.items():
                if k=="Custom":
                    continue
                b = DraggableButton(self, k, k)
                new_widgets.append(b)
                b.setIcon(UIUtils.themed_icon(v["icon"]))

                # Tooltip surfaces the direct hotkey (if any) for discoverability.
                # Buttons without a hotkey get no tooltip — keeps things uncluttered.
                hotkey = (v.get("hotkey") or "").strip()
                if hotkey:
                    b.setToolTip(f"Direct hotkey: {hotkey}")

                if not self.edit_mode:
                    b.clicked.connect(partial(self.on_generic_instruction, k))
        except Exception:
            # Leave the current buttons in place rather than a partial grid.
            for button in new_widgets:
                button.hide()
                button.deleteLater()
            raise

        old_widgets, self.button_widgets = self.button_widgets, new_widgets

        # Rebuilding used to be followed immediately by terminating the app,
        # which hid these stale widgets. Live editing must dispose of them so
        # repeated changes do not leak controls or connected signals.
        for old_button in old_widgets:
            old_button.hide()
            old_button.deleteLater()

        if self.edit_mode:
            for button in self.button_widgets:
                self.add_edit_delete_icons(button)

    def rebuild_grid_layout(self, parent_layout=None):
        """Rebuild grid layout with consistent sizing and proper Add New button placement."""
        self.clear_drop_indicator()
        if not parent_layout:
            parent_layout = self.background.layout()

        # Remove existing grid and Add New button
        for i in reversed(range(parent_layout.count())):
            item = parent_layout.itemAt(i)
            if isinstance(item, QtWidgets.QGridLayout):
                grid = item
                for j in reversed(range(grid.count())):
                    w = grid.itemAt(j).widget()
                    if w:
                        grid.removeWidget(w)
                parent_layout.removeItem(grid)
                grid.deleteLater()
            elif (item.widget() and isinstance(item.widget(), QPushButton) 
                and item.widget().text() == "+ Add New"):
                add_button = item.widget()
                parent_layout.removeWidget(add_button)
                add_button.hide()
                add_button.deleteLater()

        # Create new grid with fixed column width
        grid = QtWidgets.QGridLayout()
        grid.setSpacing(10)  
        grid.setColumnMinimumWidth(0, 120)
        grid.setColumnMinimumWidth(1, 120)
        
        # Add buttons to grid
        row = 0
        col = 0
        for b in self.button_widgets:
            grid.addWidget(b, row, col)
            col += 1
            if col > 1:
                col = 0
                row += 1
        
        parent_layout.addLayout(grid)
        # Attaching the grid reparents new buttons, which hides them until a
        # queued show. Show them now so the window can be fitted right away.
        for b in self.button_widgets:
            b.show()
        
        # Add New button (only in edit mode)
        if self.edit_mode:
            add_btn = QPushButton("+ Add New")
            add_btn.setStyleSheet(f"""
                QPushButton {{
                    background-color: {'#333' if colorMode=='dark' else '#e0e0e0'};
                    border: 1px solid {'#666' if colorMode=='dark' else '#ccc'};
                    border-radius: 8px;
                    padding: 10px;
                    font-size: 14px;
                    text-align: center;
                    color: {'#fff' if colorMode=='dark' else '#000'};
                    margin-top: 10px;
                }}
                QPushButton:hover {{
                    background-color: {'#444' if colorMode=='dark' else '#d0d0d0'};
                }}
            """)
            add_btn.clicked.connect(self.add_new_button_clicked)
            parent_layout.addWidget(add_btn)
            add_btn.show()

    def show_drop_indicator(self, button, side):
        """Show the one active insertion marker on ``button``'s chosen edge."""
        if self.drop_indicator_button is button and self.drop_indicator_side == side:
            return

        self.clear_drop_indicator()
        indicator_color = "#eeeeee" if colorMode == "dark" else "#777777"
        indicator_edge = "left" if side == "before" else "right"
        button.setStyleSheet(
            button.base_style
            + f"""
                QPushButton {{
                    border-{indicator_edge}: 3px solid {indicator_color};
                }}
            """
        )
        self.drop_indicator_button = button
        self.drop_indicator_side = side

    def clear_drop_indicator(self, button=None):
        """Remove the insertion marker, optionally only when owned by ``button``."""
        active_button = self.drop_indicator_button
        if button is not None and active_button is not button:
            return
        if active_button is not None:
            try:
                active_button.setStyleSheet(active_button.base_style)
            except RuntimeError:
                # Already destroyed by a rebuild that skipped clearing first.
                pass
        self.drop_indicator_button = None
        self.drop_indicator_side = None

    def add_edit_delete_icons(self, btn):
        """Add edit/delete icons as overlays with proper spacing."""
        if hasattr(btn, 'icon_container') and btn.icon_container:
            btn.icon_container.deleteLater()
        
        btn.icon_container = QtWidgets.QWidget(btn)
        btn.icon_container.setAttribute(QtCore.Qt.WA_TransparentForMouseEvents, False)
        
        btn.icon_container.setGeometry(0, 0, btn.width(), btn.height())
        
        circle_style = f"""
            QPushButton {{
                background-color: {'#666' if colorMode=='dark' else '#999'};
                border-radius: 10px;
                min-width: 16px;
                min-height: 16px;
                max-width: 16px;
                max-height: 16px;
                padding: 1px;
                margin: 0px;
            }}
            QPushButton:hover {{
                background-color: {'#888' if colorMode=='dark' else '#bbb'};
            }}
        """
        
        # Create edit icon (top-left)
        edit_btn = QPushButton(btn.icon_container)
        edit_btn.setGeometry(3, 3, 16, 16)
        edit_btn.setIcon(UIUtils.themed_icon('pencil'))
        edit_btn.setStyleSheet(circle_style)
        edit_btn.clicked.connect(partial(self.edit_button_clicked, btn))
        edit_btn.show()
        
        # Create delete icon (top-right)
        delete_btn = QPushButton(btn.icon_container)
        delete_btn.setGeometry(btn.width() - 23, 3, 16, 16)
        delete_btn.setIcon(UIUtils.themed_icon('cross'))
        delete_btn.setStyleSheet(circle_style)
        delete_btn.clicked.connect(partial(self.delete_button_clicked, btn))
        delete_btn.show()
        
        btn.icon_container.raise_()
        btn.icon_container.show()

    def toggle_edit_mode(self):
        """Toggle edit mode with improved button labels and state handling."""
        self.edit_mode = not self.edit_mode
        logging.debug(f'Edit mode toggled: {self.edit_mode}')

        if self.edit_mode:
            # Switch to edit mode:
            self.set_selected_option(None)
            icon_name = "check"
            # No text, just the check icon, a bit bigger:
            self.edit_button.setText("")
            self.edit_button.setFixedSize(36, 36)
            self.edit_button.setStyleSheet(f"""
                QPushButton {{
                    background-color: transparent;
                    border: none;
                    border-radius: 6px;
                    padding: 0px;
                }}
                QPushButton:hover {{
                    background-color: {'#333' if colorMode=='dark' else '#ebebeb'};
                }}
            """)
            # Hide close, show reset button & drag label
            self.close_button.hide()
            self.reset_button.show()
            self.drag_label.show()

        else:
            # Switch back to normal (non-edit) mode:
            icon_name = "pencil"
            self.edit_button.setText("")
            self.edit_button.setFixedSize(24, 24)  # Return to normal size
            # Show close, hide reset & drag label
            self.close_button.show()
            self.reset_button.hide()
            self.drag_label.hide()


        # Update the edit button icon now that icon_name is defined
        self.edit_button.setIcon(UIUtils.themed_icon(icon_name))

        # Toggle the main input area
        self.input_area.setVisible(not self.edit_mode)

        # Update button overlays. The loop resets every stylesheet below, so
        # drop the indicator first rather than leaving it tracked but invisible.
        self.clear_drop_indicator()
        for btn in self.button_widgets:
            if not self.edit_mode:
                btn.clicked.connect(partial(self.on_generic_instruction, btn.key))
                if hasattr(btn, 'icon_container') and btn.icon_container:
                    btn.icon_container.deleteLater()
                    btn.icon_container = None
            else:
                try:
                    btn.clicked.disconnect()
                except RuntimeError:
                    pass
                self.add_edit_delete_icons(btn)

            btn.setStyleSheet(btn.base_style)

        # Rebuild grid layout
        self.rebuild_grid_layout()
        self._fit_to_contents()
        if not self.edit_mode:
            self.custom_input.setFocus()


    def on_reset_clicked(self):
        """
        Reset `options.json` to the tracked defaults and apply them immediately.
        """
        confirm_box = QtWidgets.QMessageBox()
        confirm_box.setWindowTitle("Confirm Reset to Defaults?")
        confirm_box.setText(
            "Reset all buttons to their original configuration? "
            "This will replace your custom buttons and edits."
        )
        confirm_box.setStandardButtons(QtWidgets.QMessageBox.Yes | QtWidgets.QMessageBox.No)
        confirm_box.setDefaultButton(QtWidgets.QMessageBox.No)
        
        if confirm_box.exec_() == QtWidgets.QMessageBox.Yes:
            try:
                logging.debug('Resetting to default options.json')
                data = reset_options_file()
                self._activate_options(data)
            
            except Exception as e:
                logging.error(f"Error resetting options.json: {e}")
                self._show_options_error("resetting the buttons", e)

    def _validate_name(self, name, exclude_button=None):
        """
        Check a button name. Names key options.json, so a duplicate would
        silently replace another button and "Custom" would replace the
        typed-change prompt. Returns (ok, error_message).

        `exclude_button` is the button being edited, which may keep its name.
        """
        if not name:
            return False, "Please give the button a name."
        if name == "Custom":
            return False, (
                "'Custom' is reserved for changes typed into the popup. "
                "Pick a different name."
            )
        if name != exclude_button and name in self.load_options():
            return False, (
                f"A button named '{name}' already exists. Pick a different name."
            )
        return True, None

    def _validate_hotkey(self, hotkey, exclude_button=None):
        """
        Check a button hotkey string for validity and conflicts.

        Returns (ok, error_message). Empty hotkey is always ok — the dialog
        omits the field on save, which means "no direct hotkey for this
        button". This is also how every legacy/old options.json entry
        looks, so absence is always safe.

        `exclude_button` skips the named button when checking conflicts —
        used during edit so a button doesn't conflict with its own
        previously-saved hotkey.
        """
        if not hotkey:
            return True, None

        ok, problem = validate_trigger(hotkey)
        if not ok:
            return False, (
                f"Invalid shortcut: '{hotkey}'.\n\n{problem}"
            )

        # Conflict with the global Writing Tools shortcut. Same combination
        # can't dispatch to both the popup and a direct fire.
        global_shortcut = (self.app.config.get('shortcut') or 'ctrl+space').strip().lower()
        if hotkey == global_shortcut:
            return False, (
                f"'{hotkey}' is already used as the main Writing Tools "
                f"hotkey (set in Settings). Pick a different combination."
            )

        # Conflict with another button's hotkey.
        data = self.load_options()
        for k, v in data.items():
            if k == exclude_button:
                continue
            other = (v.get('hotkey') or '').strip().lower()
            if other and other == hotkey:
                return False, (
                    f"'{hotkey}' is already used by the '{k}' button. "
                    f"Pick a different combination."
                )

        return True, None

    @staticmethod
    def _build_button_entry(bd, existing=None):
        """
        Assemble the options.json entry for a button from dialog output.
        Preserves any non-dialog fields already on the existing entry, and
        only writes `hotkey` when the user provided one (legacy-clean).
        """
        entry = dict(existing) if existing else {}
        entry["prefix"] = bd["prefix"]
        entry["instruction"] = bd["instruction"]
        entry["icon"] = bd["icon"]
        entry["open_in_window"] = bd["open_in_window"]
        if bd.get("hotkey"):
            entry["hotkey"] = bd["hotkey"]
        else:
            # User cleared the hotkey field — drop the key so re-saving
            # doesn't leave a stale binding behind.
            entry.pop("hotkey", None)
        return entry

    def _fit_to_contents(self):
        """
        Resize to the current contents. A top-level window grows with its
        layout but never shrinks on its own, so removed buttons or leaving
        edit mode would otherwise leave empty space.
        """
        # Activate the inner layout first; otherwise adjustSize() reads the
        # background's size hint cached from before this change.
        self.background.layout().activate()
        self.adjustSize()

    def _show_options_error(self, action, error):
        error_msg = QtWidgets.QMessageBox(self)
        error_msg.setWindowTitle("Error")
        error_msg.setText(f"An error occurred while {action}: {error}")
        error_msg.exec_()

    def _activate_options(self, data, rebuild=True):
        """Make already-persisted options live in the app and this popup."""
        self.app.apply_options(data)
        if rebuild:
            self.build_buttons_list(data)
            self.rebuild_grid_layout()
            self._fit_to_contents()

    def _commit_options(self, data, rebuild=True):
        """Persist and activate one editor change without closing the app."""
        try:
            self.save_options(data)
        except Exception as error:
            logging.error("Error saving options.json: %s", error)
            self._show_options_error("saving the button changes", error)
            return False

        self._activate_options(data, rebuild=rebuild)
        return True

    def add_new_button_clicked(self):
        dialog = ButtonEditDialog(self, title="Add New Button")
        while dialog.exec_():
            bd = dialog.get_button_data()
            ok, err = self._validate_name(bd["name"])
            if not ok:
                QtWidgets.QMessageBox.warning(self, "Invalid name", err)
                continue
            ok, err = self._validate_hotkey(bd.get("hotkey", ""))
            if not ok:
                QtWidgets.QMessageBox.warning(self, "Invalid hotkey", err)
                # Re-open the dialog with the user's entries preserved so
                # they can fix the hotkey instead of starting over.
                continue
            data = self.load_options()
            data[bd["name"]] = self._build_button_entry(bd)
            # A failed save already showed its error; keep the entries.
            if self._commit_options(data):
                return


    def edit_button_clicked(self, btn):
        """User clicked the small pencil icon over a button."""
        key = btn.key
        data = self.load_options()
        bd = dict(data[key])
        bd["name"] = key

        dialog = ButtonEditDialog(self, bd)
        while dialog.exec_():
            new_data = dialog.get_button_data()
            new_name = new_data["name"]
            # Pass `exclude_button=key` so we don't flag the button's own
            # current name or hotkey as a conflict with itself.
            ok, err = self._validate_name(new_name, exclude_button=key)
            if not ok:
                QtWidgets.QMessageBox.warning(self, "Invalid name", err)
                continue
            ok, err = self._validate_hotkey(new_data.get("hotkey", ""), exclude_button=key)
            if not ok:
                QtWidgets.QMessageBox.warning(self, "Invalid hotkey", err)
                continue
            data = self.load_options()
            existing = data.get(key)
            if new_name != key:
                # Rename in place so the button keeps its grid position.
                data = {(new_name if k == key else k): v for k, v in data.items()}
            data[new_name] = self._build_button_entry(new_data, existing=existing)
            # A failed save already showed its error; keep the entries.
            if self._commit_options(data):
                return

    def delete_button_clicked(self, btn):
        """Handle deletion of a button."""
        key = btn.key
        confirm = QtWidgets.QMessageBox()
        confirm.setWindowTitle("Confirm Delete?")
        confirm.setText(f"Delete the '{key}' button?")
        confirm.setStandardButtons(QtWidgets.QMessageBox.Yes | QtWidgets.QMessageBox.No)
        confirm.setDefaultButton(QtWidgets.QMessageBox.No)
        
        if confirm.exec_() == QtWidgets.QMessageBox.Yes:
            try:
                data = self.load_options()
                del data[key]
                self._commit_options(data)
                
            except Exception as e:
                logging.error(f"Error deleting button: {e}")
                self._show_options_error("deleting the button", e)

    def update_json_from_grid(self):
        """
        Called after a drop reorder. Reflect the new order in options.json,
        so that user's custom arrangement persists.
        """
        try:
            data = self.load_options()
            new_data = {"Custom": data["Custom"]} if "Custom" in data else {}
            for b in self.button_widgets:
                new_data[b.key] = data[b.key]
        except Exception as error:
            # e.g. unreadable JSON, or a button removed from the file meanwhile.
            logging.error("Error reading options.json to reorder buttons: %s", error)
            self._show_options_error("saving the button order", error)
            return False
        return self._commit_options(new_data, rebuild=False)

    def on_custom_change(self):
        txt = self.custom_input.text().strip()
        # `is not None`: an empty button name is still a valid selection.
        if self.selected_option is not None:
            self.app.process_option(self.selected_option, txt or None)
            self.close()
        elif txt:
            self.app.process_option('Custom', txt)
            self.close()

    def set_selected_option(self, option):
        """Highlight `option` (or clear with None) and retitle the input."""
        self.selected_option = option
        for button in self.button_widgets:
            button.setProperty("selected", option is not None and button.key == option)
            button.style().unpolish(button)
            button.style().polish(button)

        # The highlighted button already names the action, so the placeholder
        # stays short enough not to clip "(optional)" in the narrow field.
        if option is None:
            self.custom_input.setPlaceholderText(_("Describe your change..."))
        else:
            self.custom_input.setPlaceholderText(_("Add instructions (optional)..."))

    def on_generic_instruction(self, instruction):
        if not self.edit_mode:
            # Clicking the selected button again returns to a custom change.
            if self.selected_option == instruction:
                self.set_selected_option(None)
            else:
                self.set_selected_option(instruction)
            self.custom_input.setFocus()

    # No hide-on-deactivate: under a focus-follows-mouse compositor merely
    # moving the pointer away would dismiss the popup. It closes on Escape,
    # the close button, submitting an action, or pressing the hotkey again.

    def keyPressEvent(self, event):
        if event.key()==QtCore.Qt.Key_Escape:
            self.close()
        else:
            super().keyPressEvent(event)
