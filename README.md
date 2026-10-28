# Corral

English | [Русский](README.ru.md)

Groups for the icons on your Windows desktop.

Icons get sorted into areas right on the desktop. The files themselves don't move anywhere, they stay in your Desktop folder. Corral just shows them inside a group.

The interface is in Russian for now. Menu items below are given in Russian with a translation.

## Install

Grab the zip from the [releases page](https://github.com/theilyakost/corral/releases), unpack it anywhere and run `corral.exe`. Keep `corral_agent.dll` next to it. You'll get a tray icon and a couple of example groups on the desktop.

To start it with Windows, open the settings (left click on the tray icon) and tick «Запускать при входе в Windows» (run at sign-in). Corral copies itself to `%LOCALAPPDATA%\Programs\Corral` and adds a Task Scheduler entry.

If you already have some other app that arranges desktop icons, close it first. Two of them will fight over the icons and nobody wins.

## What you can do

Drag a group by its header to move it, drag the right or bottom edge to resize. Everything snaps to the icon grid, so it lines up on its own.

You can drop icons into a group, or give it a rule like `*.pdf` and every PDF lands there by itself. To take an icon out, just drag it back onto the desktop.

Right click a group's header and there's «Режим плашки» (plaque mode): the group folds into just its header and opens when you hover over it. That's how I like it, the desktop stays clean.

There are also portals. A portal is a group that shows what's inside some folder, your work folder for example. You can open, rename and drag files in it like in Explorer.

Everything else is in the settings: themes (about twenty of them), colors, transparency, rounded corners.

The config lives in `%APPDATA%\corral\config.toml`, a plain text file. You can edit it by hand and then pick «Перечитать конфиг» (reload config) in the tray menu.

## About `corral_agent.dll`

So an icon doesn't show up twice, once in the group and once on the desktop itself, the original has to be hidden. Explorer can't do that on its own, so Corral loads a small module into it. The module doesn't patch anything, it only stops Explorer from drawing hidden icons and from reacting to clicks on them. When Corral closes, the module unloads and everything goes back to how it was.

## When it won't work

- Alongside other apps that move desktop icons around.
- There's no blur behind groups and there won't be, Windows doesn't allow it for this kind of window.
- The new Windows 11 context menu doesn't work well yet, so the classic one is shown by default. You can turn the new one on in the settings at your own risk.
- I use Windows 11. Windows 10 should work, but I haven't tried it.

## Build it yourself

You need Rust.

```
cargo build --release
```

Both files end up in `target\release`.

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0). You can use it, share it and change it, as long as it's not for money. You can't sell Corral or make money from it. If you pass it on, keep the [LICENSE](LICENSE) file with it.
