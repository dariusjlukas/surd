// Download helper for surd-data exports (workspace variables → .json file).
// The file content itself is produced by the engine (Session.export_data);
// saving it goes through the platform shim so the desktop build gets a native
// Save dialog instead of the browser's blob/anchor download.

import { saveTextFile } from '../platform/desktop'

export function downloadDataFile(fileText: string, baseName: string) {
  const name = `${baseName.replace(/[/\\:*?"<>|]/g, '_')}.data.json`
  void saveTextFile(name, fileText).catch((e) =>
    console.error('data export failed', e),
  )
}
