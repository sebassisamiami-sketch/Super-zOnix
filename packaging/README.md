# Packaging (Phase 5)

zOnix targets Linux Nobara (Fedora base) first.

| Path | Format | Notes |
|------|--------|-------|
| `rpm/zonix.spec` | native `.rpm` | Preferred on Nobara/Fedora. Depends on `pipewire`, `wireplumber`, `rnnoise`. Ships `zonixd`, `zonixctl`, the GUI, the systemd *user* unit, and the WirePlumber scripts. |
| `flatpak/dev.zonix.Zonix.yaml` | Flatpak manifest | Portable fallback. Requires `--socket=pipewire`, `--filesystem=xdg-run/pipewire-0`, and `--talk-name=org.freedesktop.portal.*` for game/foreground detection. |

Build steps (exact `rpmbuild` / `flatpak-builder` invocations and the Nobara
dependency list) are documented in `docs/ARCHITECTURE.md` and finalized in
Phase 5.
