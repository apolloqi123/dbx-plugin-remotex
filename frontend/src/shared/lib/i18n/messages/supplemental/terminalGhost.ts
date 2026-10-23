// terminalGhostMessages supplemental messages.
const terminalGhostMessages: Record<string, Record<string, string>> = {
  en: {
    "terminalGhost.sectionTitle": "Inline autosuggestions",
    "terminalGhost.label": "Inline ghost autosuggestions",
    "terminalGhost.tabAccept": "Use Tab to accept",
    "terminalGhost.tabAcceptHint": "On: Tab accepts the inline suggestion instead of the shell completion. The completion menu keeps Tab for its own candidates.",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E accept · Ctrl+→ accepts one word",
    "terminalGhost.hint": "While typing at the end of the prompt line, the remaining part of a matching history or quick command is shown in gray after the cursor. Press → to accept it in one step. Hidden while a remote command is running, during IME composition or while pasting.",
  },
  es: {
    "terminalGhost.sectionTitle": "Sugerencias en línea",
    "terminalGhost.label": "Autosugerencia en línea (fantasma)",
    "terminalGhost.tabAccept": "Usar Tab para aceptar",
    "terminalGhost.tabAcceptHint": "Activado: Tab acepta la sugerencia en línea en vez del completado del shell. El menú de completado conserva Tab para sus candidatos.",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E aceptan · Ctrl+→ palabra a palabra",
    "terminalGhost.hint": "Al escribir al final de la línea del indicador, el resto del comando coincidente del historial o de los comandos rápidos se muestra en gris tras el cursor. Pulsa → para aceptarlo de una vez. No aparece mientras se ejecuta un comando remoto, durante la composición del IME ni al pegar.",
  },
  it: {
    "terminalGhost.sectionTitle": "Suggerimenti in linea",
    "terminalGhost.label": "Suggerimenti automatici in linea (ghost)",
    "terminalGhost.tabAccept": "Usa Tab per accettare",
    "terminalGhost.tabAcceptHint": "Attivo: Tab accetta il suggerimento inline invece del completamento della shell. Il menu di completamento mantiene Tab per i propri candidati.",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E accettano · Ctrl+→ parola per parola",
    "terminalGhost.hint": "Mentre digiti alla fine della riga del prompt, la parte rimanente di un comando corrispondente (cronologia o comandi rapidi) appare in grigio dopo il cursore. Premi → per accettarlo in un passaggio. Nascosto durante l'esecuzione di un comando remoto, con la composizione IME o durante l'incolla.",
  },
  ja: {
    "terminalGhost.sectionTitle": "行内自動補完",
    "terminalGhost.label": "行内ゴースト自動補完",
    "terminalGhost.tabAccept": "Tab で確定",
    "terminalGhost.tabAcceptHint": "オンにすると、Tab がシェル補完の代わりに行内候補を確定します。補完メニューが開いている間は Tab はメニュー候補に使われます。",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E で確定 · Ctrl+→ で単語単位",
    "terminalGhost.hint": "プロンプト行の末尾で入力すると、履歴とクイックコマンドから一致する残り部分が灰色でカーソルの後に表示されます。→ で一括確定。リモートコマンドの実行中、IME 変換中、貼り付け中は表示されません。",
  },
  "pt-BR": {
    "terminalGhost.sectionTitle": "Sugestões em linha",
    "terminalGhost.label": "Sugestão automática em linha (fantasma)",
    "terminalGhost.tabAccept": "Usar Tab para aceitar",
    "terminalGhost.tabAcceptHint": "Ativado: Tab aceita a sugestão em linha em vez do completamento do shell. O menu de completamento mantém Tab para seus candidatos.",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E aceitam · Ctrl+→ palavra por palavra",
    "terminalGhost.hint": "Ao digitar no fim da linha do prompt, o restante do comando correspondente do histórico ou dos comandos rápidos aparece em cinza após o cursor. Pressione → para aceitar de uma vez. Não aparece enquanto um comando remoto está em execução, durante a composição do IME ou ao colar.",
  },
  "zh-CN": {
    "terminalGhost.sectionTitle": "行内自动建议",
    "terminalGhost.label": "行内自动建议（ghost）",
    "terminalGhost.tabAccept": "用 Tab 接受建议",
    "terminalGhost.tabAcceptHint": "开启后 Tab 改为接受行内建议（而非交给远端 shell 补全）；结构化补全菜单开着时 Tab 仍归菜单候选。",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E 整段接受 · Ctrl+→ 逐词接受",
    "terminalGhost.hint": "在提示符行尾键入时，以灰色在光标后内联展示历史与快速命令中匹配项的剩余部分，按 → 一次接受；远端命令执行中、输入法组合中或粘贴时不出现。",
  },
  "zh-TW": {
    "terminalGhost.sectionTitle": "行內自動建議",
    "terminalGhost.label": "行內自動建議（ghost）",
    "terminalGhost.tabAccept": "用 Tab 接受建議",
    "terminalGhost.tabAcceptHint": "開啟後 Tab 改為接受行內建議（而非交給遠端 shell 補全）；結構化補全選單開啟時 Tab 仍歸選單候選。",
    "terminalGhost.keysSummary": "→ / Ctrl+F / Ctrl+E 整段接受 · Ctrl+→ 逐詞接受",
    "terminalGhost.hint": "在提示字元行尾鍵入時，以灰色在游標後內聯顯示歷史與快速命令中相符項的剩餘部分，按 → 一次接受；遠端命令執行中、輸入法組字中或貼上時不出現。",
  },
};

export default terminalGhostMessages;
