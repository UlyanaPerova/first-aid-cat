// First Aid Cat (Аптечка) — AI limits icon in the top bar.
// The app writes $XDG_RUNTIME_DIR/aptechka/tray.json ({label, icon, panel, visible, at}) and a ready
// picture of its limits panel. Like other top-bar indicators, the panel is a normal GNOME menu: one click
// opens it at once, the next click (or a click elsewhere) closes it. The «Usage chart…» button on the
// picture asks the app (session bus, io.github.aptechka.Tray) to open its usage chart.
// While the menu is open the scene at the bottom of the picture is alive: the app is told (PanelOpen) and
// writes a small picture of the current frame into tray.json ({scene: {path, x, y, w, h}}); a second actor
// shows it exactly over the static scene. An app without PanelOpen simply leaves the static picture.
import GObject from 'gi://GObject';
import St from 'gi://St';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import GdkPixbuf from 'gi://GdkPixbuf';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS = 'io.github.aptechka.Tray';
const PATH = '/io/github/aptechka/Tray';
const IFACE = `<node><interface name="io.github.aptechka.Tray">
  <method name="OpenAnalytics"/>
  <method name="PanelOpen"><arg type="b" direction="in" name="open"/></method>
  <method name="PanelClick"><arg type="i" direction="in" name="x"/><arg type="i" direction="in" name="y"/></method>
</interface></node>`;
const TrayProxy = Gio.DBusProxy.makeProxyWrapper(IFACE);
// the app updates the file every few seconds; older than this means the app is gone
const STALE_S = 120;

const Indicator = GObject.registerClass(
class AptechkaIndicator extends PanelMenu.Button {
    _init() {
        super._init(0.5, 'First Aid Cat', false);
        const box = new St.BoxLayout({style_class: 'panel-status-menu-box'});
        this._icon = new St.Icon({style_class: 'system-status-icon'});
        this._label = new St.Label({y_align: Clutter.ActorAlign.CENTER, style: 'margin-left: 6px;'});
        box.add_child(this._icon);
        box.add_child(this._label);
        this.add_child(box);

        // the panel picture inside the menu
        this._panel = new St.Widget({reactive: true, layout_manager: new Clutter.FixedLayout()});
        // the live scene lies over the static one (not reactive: clicks go to the panel)
        this._scene = new St.Widget({reactive: false, visible: false});
        this._scenePath = '';
        this._monitor = null;
        this._panel.add_child(this._scene);
        this._panel.connect('button-release-event', (_a, event) => this._onPanelClick(event));
        this.menu.box.add_child(this._panel);
        // только плашка: без тёмного фона, рамки, отступов и стрелки стандартного меню
        this.menu.box.style = 'padding: 0; margin: 0; spacing: 0;';
        this.menu.actor.add_style_class_name('aptechka-panel-menu');
        this.menu.actor.style = '-arrow-background-color: transparent; -arrow-border-width: 0; -arrow-border-color: transparent; -arrow-base: 0; -arrow-rise: 0; -boxpointer-gap: 2px;';
        const content = this.menu.box.get_parent();
        if (content)
            content.style = 'background-color: transparent; padding: 0; margin: 0; border: none; box-shadow: none; border-radius: 0;';
        this.menu.connect('open-state-changed', (_m, open) => {
            if (open)
                this._refresh();
            this._setLive(open);
        });

        this._file = Gio.File.new_for_path(GLib.build_filenamev([GLib.get_user_runtime_dir(), 'aptechka', 'tray.json']));
        this._iconPath = '';
        this._panelInfo = null;
        this._proxy = null;
        this._refresh();
        this._timerId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 2, () => {
            this._refresh();
            return GLib.SOURCE_CONTINUE;
        });
    }

    _onPanelClick(event) {
        const p = this._panelInfo;
        if (!p)
            return Clutter.EVENT_PROPAGATE;
        const [ex, ey] = event.get_coords();
        const [ax, ay] = this._panel.get_transformed_position();
        const [aw, ah] = this._panel.get_transformed_size();
        // from screen coordinates to picture pixels (works at any display scale)
        const x = (ex - ax) / aw * p.w;
        const y = (ey - ay) / ah * p.h;
        const inside = r => r && x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
        if (inside(p.btn)) {
            this.menu.close();
            this._call(proxy => proxy.OpenAnalyticsRemote(() => {}));
        } else if (inside(p.close)) {
            this.menu.close();
        } else {
            // anywhere else on the picture: the app decides (a click on the cat's ear makes it twitch);
            // an app without PanelClick simply ignores the call
            this._call(proxy => proxy.PanelClickRemote(Math.floor(x), Math.floor(y), () => {}));
        }
        return Clutter.EVENT_STOP;
    }

    _call(fn) {
        if (this._proxy) {
            fn(this._proxy);
            return;
        }
        new TrayProxy(Gio.DBus.session, BUS, PATH, (proxy, err) => {
            if (err)
                return;
            this._proxy = proxy;
            fn(proxy);
        });
    }

    // Menu open: tell the app and watch tray.json for new scene frames; closed: stop and hide the scene.
    _setLive(open) {
        try {
            this._call(proxy => proxy.PanelOpenRemote(open, () => {}));
        } catch (e) {
            // an app without PanelOpen: the picture stays static
        }
        if (open && !this._monitor) {
            try {
                this._monitor = this._file.monitor_file(Gio.FileMonitorFlags.NONE, null);
                this._monitorId = this._monitor.connect('changed', () => this._refresh(true));
            } catch (e) {
                this._monitor = null;
            }
        } else if (!open) {
            if (this._monitor) {
                this._monitor.disconnect(this._monitorId);
                this._monitor.cancel();
                this._monitor = null;
            }
            this._scenePath = '';
            this._scene.visible = false;
            this._scene.set_content(null);
        }
    }

    // Put the current scene frame (a 1x picture) over the static scene: stretched by whole pixels, NEAREST.
    _updateScene(s) {
        if (!this.menu.isOpen || !s || !s.path) {
            this._scene.visible = false;
            return;
        }
        if (s.path !== this._scenePath) {
            try {
                const pixbuf = GdkPixbuf.Pixbuf.new_from_file(s.path);
                const [w, h] = [pixbuf.get_width(), pixbuf.get_height()];
                const format = pixbuf.get_has_alpha() ? Cogl.PixelFormat.RGBA_8888 : Cogl.PixelFormat.RGB_888;
                const content = St.ImageContent.new_with_preferred_size(w, h);
                const args = [pixbuf.read_pixel_bytes(), format, w, h, pixbuf.get_rowstride()];
                try {
                    content.set_bytes(...args);
                } catch (e) {
                    content.set_bytes(global.stage.context.get_backend().get_cogl_context(), ...args);
                }
                this._scene.set_content(content);
                this._scene.set_content_scaling_filters(Clutter.ScalingFilter.NEAREST, Clutter.ScalingFilter.NEAREST);
                this._scenePath = s.path;
            } catch (e) {
                // frame not readable (already replaced): wait for the next one
                return;
            }
        }
        this._scene.set_position(s.x, s.y);
        this._scene.set_size(s.w, s.h);
        this._scene.visible = true;
    }

    _refresh(sceneOnly = false) {
        let data = null;
        try {
            const [ok, bytes] = this._file.load_contents(null);
            if (ok)
                data = JSON.parse(new TextDecoder().decode(bytes));
        } catch (e) {
            data = null;
        }
        if (sceneOnly) {
            if (data)
                this._updateScene(data.scene);
            return;
        }
        const now = GLib.get_real_time() / 1e6;
        this.visible = !!(data && data.visible && now - (data.at || 0) < STALE_S);
        if (!this.visible)
            return;
        this._label.text = data.label || '';
        if (data.icon && data.icon !== this._iconPath) {
            this._iconPath = data.icon;
            this._icon.gicon = Gio.FileIcon.new(Gio.File.new_for_path(data.icon));
        }
        const p = data.panel;
        if (p && p.path && (!this._panelInfo || this._panelInfo.path !== p.path || (this._panelInfo.at !== data.at && !this.menu.isOpen))) {
            this._panelInfo = {...p, at: data.at};
            this._showPanel(p);
        }
        this._updateScene(data.scene);
    }

    // The picture is pixel art drawn by the app in screen pixels. Show it dot for dot: its own pixels as the
    // actor's content, actor size in stage (= screen) pixels, nearest-neighbour filter. A CSS background-image
    // is wrong for this: at display scale 2 St treats the file as a 1x image, upscales it with smoothing and
    // scales it back down, which blurs every pixel edge.
    _showPanel(p) {
        try {
            const pixbuf = GdkPixbuf.Pixbuf.new_from_file(p.path);
            const [w, h] = [pixbuf.get_width(), pixbuf.get_height()];
            const format = pixbuf.get_has_alpha() ? Cogl.PixelFormat.RGBA_8888 : Cogl.PixelFormat.RGB_888;
            const content = St.ImageContent.new_with_preferred_size(w, h);
            const args = [pixbuf.read_pixel_bytes(), format, w, h, pixbuf.get_rowstride()];
            try {
                content.set_bytes(...args);
            } catch (e) {
                // GNOME 48+: the Cogl context comes first
                content.set_bytes(global.stage.context.get_backend().get_cogl_context(), ...args);
            }
            this._panel.style = '';
            this._panel.set_content(content);
            this._panel.set_content_scaling_filters(Clutter.ScalingFilter.NEAREST, Clutter.ScalingFilter.NEAREST);
            this._panel.set_size(w, h);
        } catch (e) {
            // could not load the pixels: the old way (smoothed, but the panel is there)
            const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor || 1;
            const w = Math.round(p.w / scale), h = Math.round(p.h / scale);
            this._panel.set_content(null);
            this._panel.set_size(-1, -1);
            this._panel.style = `background-image: url("file://${p.path}"); background-size: ${w}px ${h}px; width: ${w}px; height: ${h}px;`;
        }
    }

    destroy() {
        this._setLive(false);
        if (this._timerId)
            GLib.source_remove(this._timerId);
        this._timerId = 0;
        super.destroy();
    }
});

export default class AptechkaLimitsExtension extends Extension {
    enable() {
        this._indicator = new Indicator();
        Main.panel.addToStatusArea(this.uuid, this._indicator, 0, 'right');
        // the app sees this name and stops showing its AppIndicator icon (no double icon)
        this._ownId = Gio.bus_own_name(Gio.BusType.SESSION, 'io.github.aptechka.ShellIcon', Gio.BusNameOwnerFlags.NONE, null, null, null);
    }

    disable() {
        if (this._ownId)
            Gio.bus_unown_name(this._ownId);
        this._ownId = 0;
        this._indicator?.destroy();
        this._indicator = null;
    }
}
