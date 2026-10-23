// x11Messages supplemental messages.
const x11Messages: Record<string, Record<string, string>> = {
  en: {
    "x11.sectionTitle": "X11 forwarding",
    "x11.enabled": "Enable X11 forwarding",
    "x11.enabledHint": "Needs a local X server (macOS: XQuartz; Windows: VcXsrv). Takes effect for sessions opened after enabling. Use only on trusted networks.",
    "x11.readOnlyNote": "X11 forwarding is disabled on read-only connections.",
  },
  "zh-CN": {
    "x11.sectionTitle": "X11 转发",
    "x11.enabled": "启用 X11 转发",
    "x11.enabledHint": "需要本机 X server（macOS：XQuartz；Windows：VcXsrv）。对开启之后新建的会话生效。请仅在可信网络使用。",
    "x11.readOnlyNote": "只读连接上 X11 转发不可用。",
  },
  "zh-TW": {
    "x11.sectionTitle": "X11 轉發",
    "x11.enabled": "啟用 X11 轉發",
    "x11.enabledHint": "需要本機 X server（macOS：XQuartz；Windows：VcXsrv）。對開啟之後新建的連線生效。請僅在可信網路使用。",
    "x11.readOnlyNote": "唯讀連線上 X11 轉發不可用。",
  },
  es: {
    "x11.sectionTitle": "Reenvío X11",
    "x11.enabled": "Activar reenvío X11",
    "x11.enabledHint": "Requiere un servidor X local (macOS: XQuartz; Windows: VcXsrv). Surte efecto en las sesiones abiertas tras activarlo. Úsalo solo en redes de confianza.",
    "x11.readOnlyNote": "El reenvío X11 no está disponible en conexiones de solo lectura.",
  },
  it: {
    "x11.sectionTitle": "Inoltro X11",
    "x11.enabled": "Abilita inoltro X11",
    "x11.enabledHint": "Richiede un server X locale (macOS: XQuartz; Windows: VcXsrv). Ha effetto sulle sessioni aperte dopo l'attivazione. Usare solo su reti affidabili.",
    "x11.readOnlyNote": "L'inoltro X11 non è disponibile sulle connessioni di sola lettura.",
  },
  ja: {
    "x11.sectionTitle": "X11 転送",
    "x11.enabled": "X11 転送を有効化",
    "x11.enabledHint": "ローカルの X サーバーが必要です（macOS: XQuartz、Windows: VcXsrv）。有効化後に開いたセッションに反映されます。信頼できるネットワークでのみ使用してください。",
    "x11.readOnlyNote": "読み取り専用接続では X11 転送は利用できません。",
  },
  "pt-BR": {
    "x11.sectionTitle": "Encaminhamento X11",
    "x11.enabled": "Ativar encaminhamento X11",
    "x11.enabledHint": "Requer um servidor X local (macOS: XQuartz; Windows: VcXsrv). Vale para sessões abertas após ativar. Use apenas em redes confiáveis.",
    "x11.readOnlyNote": "O encaminhamento X11 não está disponível em conexões somente leitura.",
  },
};

export default x11Messages;
