# Clipboard Saver

Historial de portapapeles para macOS, escrito en Rust. Vive como ícono en la
**menu bar** (barra superior) y guarda los últimos **40** elementos copiados —
texto e imágenes — para que nada se pierda al copiar encima. Funcionalidad
análoga al clipboard de Windows (Win+V), pero nativa de Mac.

## Características

- Ícono en la menu bar con menú nativo: los últimos 40 copiados, con preview
  de texto (60 caracteres) y thumbnails para imágenes.
- Click en un ítem → vuelve al portapapeles, listo para pegar con ⌘V.
- Deduplicación: copiar algo que ya está en el historial lo sube al tope en
  lugar de duplicarlo (mismo comportamiento que Windows).
- Persistencia: el historial sobrevive reinicios. Se guarda en
  `~/Library/Application Support/clipboard_saver/` (índice JSON + PNGs).
- "Iniciar con el sistema": toggle en el menú que instala un LaunchAgent
  (`~/Library/LaunchAgents/com.luiss0606.clipboard-saver.plist`) para que la
  app arranque sola al prender o reiniciar la Mac. Toma efecto en el próximo
  inicio de sesión.
- Sin ícono en el Dock: es una app de tipo agente (solo menu bar).
- **Auto-actualización**: la app revisa GitHub Releases cada 6 horas (y al
  arrancar). Cuando hay versión nueva aparece "⬇ Actualizar a vX y reiniciar"
  en el menú — un click descarga, reemplaza la app en `/Applications` y la
  relanza. Los builds de desarrollo (`cargo run`) tienen el updater
  desactivado.

## Instalación

Una línea:

```sh
curl -fsSL https://raw.githubusercontent.com/Luiss0606/clipboard_saver/main/scripts/install.sh | bash
```

El script descarga el último release, instala **Clipboard Saver.app** en
`/Applications` y la abre. Descargar con `curl` evita el atributo de
quarantine, así que Gatekeeper no muestra ningún aviso pese a la firma
ad-hoc. Después: despliega el menú 📋 y activa **Iniciar con el sistema**.
A partir de ahí la app se actualiza sola con cada release.

### Modo desarrollo

```sh
cargo run          # corre la app directamente
cargo test         # unit tests de historial, storage y menú
```

## Notas de macOS

- **Permiso de portapapeles**: en macOS 15.4+ el sistema puede mostrar un
  aviso de privacidad la primera vez que la app lee el portapapeles. Es
  esperado: leer el portapapeles es exactamente lo que hace esta app.
- **Gatekeeper**: la app se firma ad-hoc (sin cuenta de Apple Developer).
  Tanto el instalador como las auto-actualizaciones descargan sin atributo
  de quarantine, así que macOS nunca bloquea la app. Si alguna vez aparece
  el aviso "Apple could not verify…", limpiarlo con:
  `xattr -dr com.apple.quarantine "/Applications/Clipboard Saver.app"`.
- **Privacidad**: el historial se guarda en disco sin cifrar. Si copias una
  contraseña, quedará en `~/Library/Application Support/clipboard_saver/`
  hasta que salga del historial o uses **Limpiar historial**.

## Flujo de desarrollo y despliegue

```
develop ──► trabajo diario; CI corre fmt + clippy + tests (ci.yml)
   │
   └─ PR develop → main
main    ──► release.yml: tests → bundle .app → release v0.1.N (app.zip)
                  │
                  └─► la app instalada detecta el release y se auto-actualiza
```

- Commits convencionales (`feat:`, `fix:`, `ci:`, `docs:`…).
- El versionado es automático: `v0.1.N` con N = número de run de Actions.
- Convención completa en [.claude/skills/release-flow/SKILL.md](.claude/skills/release-flow/SKILL.md).

## Arquitectura

| Módulo | Responsabilidad |
| --- | --- |
| `watcher.rs` | Detecta cambios vía `NSPasteboard.changeCount` (polling 400ms) y lee texto/imagen con `arboard` |
| `history.rs` | Cola de 40 ítems, dedupe por contenido, promoción al tope |
| `storage.rs` | Persistencia: `history.json` + imágenes PNG |
| `menu.rs` | Construye el menú nativo (muda) desde el historial |
| `autostart.rs` | LaunchAgent para inicio automático |
| `updater.rs` | Auto-update desde GitHub Releases (check 6h, swap de .app, relaunch) |
| `main.rs` | Event loop (tao), tray icon, wiring |
