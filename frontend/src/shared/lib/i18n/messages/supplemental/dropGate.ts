// dropGateMessages supplemental messages.
const dropGateMessages: Record<string, Record<string, string>> = {
  "zh-CN": {
    dropRefused: "已忽略拖入的文件：会话未连接、为只读或正在传输中",
    terminalDropToPanel: "SFTP 面板已打开：请把文件拖到 SFTP 面板上传",
  },
  "zh-TW": {
    dropRefused: "已忽略拖入的檔案：工作階段未連線、為唯讀或正在傳輸中",
    terminalDropToPanel: "SFTP 面板已開啟：請把檔案拖到 SFTP 面板上傳",
  },
  en: {
    dropRefused: "Dropped files ignored: the session is not connected, is read-only, or a transfer is in progress",
    terminalDropToPanel: "The SFTP panel is open — drop files onto the SFTP panel to upload",
  },
  es: {
    dropRefused: "Archivos soltados ignorados: la sesión no está conectada, es de solo lectura o hay una transferencia en curso",
    terminalDropToPanel: "El panel SFTP está abierto: suelta los archivos en el panel SFTP para subirlos",
  },
  it: {
    dropRefused: "File rilasciati ignorati: la sessione non è connessa, è in sola lettura o è in corso un trasferimento",
    terminalDropToPanel: "Il pannello SFTP è aperto: rilascia i file sul pannello SFTP per caricarli",
  },
  ja: {
    dropRefused: "ドロップされたファイルを無視しました：セッションが未接続、読み取り専用、または転送中です",
    terminalDropToPanel: "SFTP パネルが開いています：ファイルは SFTP パネルにドロップしてアップロードしてください",
  },
  "pt-BR": {
    dropRefused: "Arquivos soltados ignorados: a sessão não está conectada, é somente leitura ou há uma transferência em andamento",
    terminalDropToPanel: "O painel SFTP está aberto — solte os arquivos no painel SFTP para enviá-los",
  },
};

export default dropGateMessages;
