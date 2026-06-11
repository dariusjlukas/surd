// Download helper for surd-data exports (workspace variables → .json file).
// The file content itself is produced by the engine (Session.export_data);
// this is just the blob/anchor dance, mirroring notebookFile.downloadNotebook.

export function downloadDataFile(fileText: string, baseName: string) {
  const blob = new Blob([fileText], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${baseName.replace(/[/\\:*?"<>|]/g, '_')}.data.json`
  a.click()
  URL.revokeObjectURL(url)
}
