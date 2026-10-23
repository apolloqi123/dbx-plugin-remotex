// historyPanelSettingsMessages supplemental messages.
const historyPanelSettingsMessages: Record<string, Record<string, string>> = {
  en: {
    "historyPanelSettings.sectionTitle": "Command history panel",
    "historyPanelSettings.enabledLabel": "Enable the command history panel",
    "historyPanelSettings.enabledHint": "On: ↑ at an idle prompt opens the command history panel, and Ctrl+R / Ctrl+Shift+H open it with search focused. Off: ↑ and Ctrl+R are passed to the remote shell, so its native history and reverse search (which record commands as completed by Tab) respond instead. Command history collection and suggestion popups are separate features and keep working.",
    "historyPanelSettings.limitLabel": "History entries to keep",
    "historyPanelSettings.limitHint": "How many commands the panel, suggestions and ghost reuse keep per connection ({min}–{max}, default {def}). Lowering the value trims existing history immediately.",
  },
  es: {
    "historyPanelSettings.sectionTitle": "Panel de historial de comandos",
    "historyPanelSettings.enabledLabel": "Activar el panel de historial de comandos",
    "historyPanelSettings.enabledHint": "Activado: ↑ con el prompt en reposo abre el panel de historial, y Ctrl+R / Ctrl+Shift+H lo abren con el buscador enfocado. Desactivado: ↑ y Ctrl+R se envían al shell remoto, que responde con su historial nativo y su búsqueda inversa (registran los comandos tal como quedaron tras el completado con Tab). La recolección de historial y las sugerencias son funciones independientes y siguen activas.",
    "historyPanelSettings.limitLabel": "Entradas de historial que se conservan",
    "historyPanelSettings.limitHint": "Cuántos comandos conservan el panel, las sugerencias y el ghost por conexión ({min}–{max}, por defecto {def}). Al reducir el valor, el historial existente se recorta de inmediato.",
  },
  it: {
    "historyPanelSettings.sectionTitle": "Pannello cronologia comandi",
    "historyPanelSettings.enabledLabel": "Attiva il pannello della cronologia comandi",
    "historyPanelSettings.enabledHint": "Attivo: ↑ con il prompt inattivo apre il pannello della cronologia, e Ctrl+R / Ctrl+Shift+H lo aprono con la ricerca attiva. Disattivo: ↑ e Ctrl+R vengono inviati alla shell remota, che risponde con la cronologia nativa e la ricerca inversa (registrano i comandi come risultano dopo il completamento con Tab). Raccolta della cronologia e suggerimenti sono funzioni indipendenti e continuano a funzionare.",
    "historyPanelSettings.limitLabel": "Voci di cronologia da conservare",
    "historyPanelSettings.limitHint": "Quanti comandi mantengono per connessione il pannello, i suggerimenti e il ghost ({min}–{max}, predefinito {def}). Riducendo il valore la cronologia esistente viene subito trimmata.",
  },
  ja: {
    "historyPanelSettings.sectionTitle": "コマンド履歴パネル",
    "historyPanelSettings.enabledLabel": "コマンド履歴パネルを有効にする",
    "historyPanelSettings.enabledHint": "オン: プロンプト待機中に ↑ でコマンド履歴パネルが開き、Ctrl+R / Ctrl+Shift+H は検索フォーカス付きで開きます。オフ: ↑ と Ctrl+R はリモートシェルに渡され、ネイティブ履歴とインクリメンタル検索（Tab 補完後のコマンドをそのまま記録）が応答します。履歴の収集とサジェストは独立した機能のため、影響を受けません。",
    "historyPanelSettings.limitLabel": "保持する履歴件数",
    "historyPanelSettings.limitHint": "接続ごとにパネル・サジェスト・ghost が保持するコマンド件数（{min}〜{max}、既定 {def}）。値を下げると既存の履歴は直ちに切り詰められます。",
  },
  "pt-BR": {
    "historyPanelSettings.sectionTitle": "Painel de histórico de comandos",
    "historyPanelSettings.enabledLabel": "Ativar o painel de histórico de comandos",
    "historyPanelSettings.enabledHint": "Ativado: ↑ com o prompt ocioso abre o painel de histórico, e Ctrl+R / Ctrl+Shift+H o abrem com a busca focada. Desativado: ↑ e Ctrl+R são enviados ao shell remoto, que responde com o histórico nativo e a busca reversa (registram os comandos como ficaram após o completado com Tab). A coleta de histórico e as sugestões são funções independentes e continuam funcionando.",
    "historyPanelSettings.limitLabel": "Entradas de histórico a manter",
    "historyPanelSettings.limitHint": "Quantos comandos o painel, as sugestões e o ghost mantêm por conexão ({min}–{max}, padrão {def}). Reduzir o valor aparar o histórico existente imediatamente.",
  },
  "zh-CN": {
    "historyPanelSettings.sectionTitle": "命令历史面板",
    "historyPanelSettings.enabledLabel": "启用命令历史面板",
    "historyPanelSettings.enabledHint": "开启：提示符空闲时按 ↑ 打开命令历史面板，Ctrl+R / Ctrl+Shift+H 打开并聚焦搜索。关闭：↑ 和 Ctrl+R 直接透传远端 shell，由其原生历史与反向搜索（记录的是 Tab 补全后实际执行的命令）响应；命令历史采集与建议浮层是独立功能，不受影响。",
    "historyPanelSettings.limitLabel": "历史记录保留条数",
    "historyPanelSettings.limitHint": "每个连接在面板、命令建议与行内 ghost 中保留的命令条数（{min}–{max}，默认 {def}）。调小后现有历史立即裁剪。",
  },
  "zh-TW": {
    "historyPanelSettings.sectionTitle": "命令歷史面板",
    "historyPanelSettings.enabledLabel": "啟用命令歷史面板",
    "historyPanelSettings.enabledHint": "開啟：提示字元閒置時按 ↑ 開啟命令歷史面板，Ctrl+R / Ctrl+Shift+H 開啟並聚焦搜尋。關閉：↑ 和 Ctrl+R 直接透傳遠端 shell，由其原生歷史與反向搜尋（記錄的是 Tab 補全後實際執行的命令）回應；命令歷史採集與建議浮層是獨立功能，不受影響。",
    "historyPanelSettings.limitLabel": "歷史記錄保留筆數",
    "historyPanelSettings.limitHint": "每個連線在面板、命令建議與行內 ghost 中保留的命令筆數（{min}–{max}，預設 {def}）。調小後現有歷史立即裁剪。",
  },
};

export default historyPanelSettingsMessages;
