//! Windows-ресурсы для clod-bridge.exe: VERSIONINFO + manifest + иконка.
//!
//! Без них exe — «голый» strip-нутый бинарь, слушающий сокет: сильнейшая
//! статическая эвристика Defender/SmartScreen (портрет малвари). Заполненные
//! метаданные издателя и asInvoker-манифест заметно снижают скоринг.

fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set(
            "FileDescription",
            "Clod Bridge — local AI proxy bridge (Clod Helper)",
        );
        res.set("ProductName", "Clod Helper Bridge");
        res.set("CompanyName", "iimperium");
        res.set("LegalCopyright", "© iimperium");
        res.set("OriginalFilename", "clod-bridge.exe");
        res.set("InternalName", "clod-bridge");
        res.set_icon("../src-tauri/icons/icon.ico");
        res.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
    </application>
  </compatibility>
</assembly>"#,
        );
        res.compile()
            .expect("winresource: не удалось вкомпилировать ресурсы");
    }
}
