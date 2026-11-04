// Docker 面板的补充文案（页头搜索、日志「加载更早」；动态键值在 base 之外单独成表，
// 避免基础 locale 文件继续膨胀；合并语义 supplemental 先写、base 后覆盖）。
const dockerPanelMessages: Record<string, Record<string, string>> = {
  en: {
    "docker.searchPlaceholder": "Search containers",
    "docker.searchEmpty": "No containers match “{query}”",
    "docker.searchClear": "Clear search",
    "docker.logsOlder": "Load earlier lines",
  },
  "zh-CN": {
    "docker.searchPlaceholder": "搜索容器",
    "docker.searchEmpty": "没有匹配「{query}」的容器",
    "docker.searchClear": "清除搜索",
    "docker.logsOlder": "加载更早",
  },
  "zh-TW": {
    "docker.searchPlaceholder": "搜尋容器",
    "docker.searchEmpty": "沒有符合「{query}」的容器",
    "docker.searchClear": "清除搜尋",
    "docker.logsOlder": "載入更早",
  },
  es: {
    "docker.searchPlaceholder": "Buscar contenedores",
    "docker.searchEmpty": "Ningún contenedor coincide con «{query}»",
    "docker.searchClear": "Borrar búsqueda",
    "docker.logsOlder": "Cargar líneas anteriores",
  },
  it: {
    "docker.searchPlaceholder": "Cerca contenitori",
    "docker.searchEmpty": "Nessun contenitore corrisponde a «{query}»",
    "docker.searchClear": "Cancella ricerca",
    "docker.logsOlder": "Carica righe precedenti",
  },
  ja: {
    "docker.searchPlaceholder": "コンテナを検索",
    "docker.searchEmpty": "「{query}」に一致するコンテナはありません",
    "docker.searchClear": "検索をクリア",
    "docker.logsOlder": "以前の行を読み込む",
  },
  "pt-BR": {
    "docker.searchPlaceholder": "Pesquisar contêineres",
    "docker.searchEmpty": "Nenhum contêiner corresponde a “{query}”",
    "docker.searchClear": "Limpar pesquisa",
    "docker.logsOlder": "Carregar linhas anteriores",
  },
};

export default dockerPanelMessages;
