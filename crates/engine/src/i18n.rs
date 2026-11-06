//! Menu labels in the Windows display language.

use std::sync::OnceLock;

use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
use windows::core::{HSTRING, PCWSTR};

/// Columns of [`STRINGS`] after the key.
const LANGS: [&str; 12] = ["en", "ru", "uk", "de", "fr", "es", "it", "pt", "pl", "zh", "ja", "ko"];

/// Primary language subtag of the Windows display language, e.g. "ru".
/// `WIDGET_LANG` overrides it: the app's language setting.
pub(crate) fn lang() -> &'static str {
    static LANG: OnceLock<String> = OnceLock::new();
    LANG.get_or_init(|| std::env::var("WIDGET_LANG").unwrap_or_else(|_| system_lang()))
}

fn system_lang() -> String {
    let mut name = [0u16; 85];
    let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut name), 0) };
    let tag = String::from_utf16_lossy(&name[..(n.max(1) - 1) as usize]);
    tag.split('-').next().unwrap_or("en").to_lowercase()
}

/// Locale for date and time texts: null (the user's formats) unless another display
/// language than Windows' is picked, then that language's ("en-US" for "en").
pub(crate) fn locale() -> PCWSTR {
    static LOCALE: OnceLock<Option<HSTRING>> = OnceLock::new();
    let l = LOCALE.get_or_init(|| {
        let lang = lang();
        let region = match lang {
            "en" => "US", "uk" => "UA", "zh" => "CN", "ja" => "JP", "ko" => "KR", "pt" => "BR",
            _ => "",
        };
        let name = if region.is_empty() { format!("{lang}-{}", lang.to_uppercase()) } else { format!("{lang}-{region}") };
        (lang != system_lang()).then(|| HSTRING::from(name))
    });
    l.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()))
}

/// `key` in the display language. Keys are the English labels, with an optional "|context"
/// where one English word needs different translations. Unknown keys come back unchanged.
pub(crate) fn t(key: &str) -> &str {
    let col = LANGS.iter().position(|l| *l == lang()).unwrap_or(0);
    match STRINGS.iter().find(|row| row[0] == key) {
        Some(row) => row[col + 1],
        None => key,
    }
}

#[rustfmt::skip]
const STRINGS: &[[&str; 13]] = &[
    // key, en, ru, uk, de, fr, es, it, pt, pl, zh, ja, ko
    ["Edit", "Edit", "Изменить", "Змінити", "Bearbeiten", "Modifier", "Editar", "Modifica", "Editar", "Edytuj", "编辑", "編集", "편집"],
    ["White|color", "White", "Белый", "Білий", "Weiß", "Blanc", "Blanco", "Bianco", "Branco", "Biały", "白色", "白", "흰색"],
    ["Black|color", "Black", "Чёрный", "Чорний", "Schwarz", "Noir", "Negro", "Nero", "Preto", "Czarny", "黑色", "黒", "검은색"],
    ["Not Playing", "Not Playing", "Ничего не играет", "Нічого не грає", "Keine Wiedergabe", "Aucune lecture", "Sin reproducción", "Nessuna riproduzione", "Nada a tocar", "Nic nie gra", "未在播放", "再生していません", "재생 중 아님"],
    ["Unknown|track", "Unknown", "Без названия", "Без назви", "Unbekannt", "Inconnu", "Desconocido", "Sconosciuto", "Desconhecido", "Nieznany", "未知", "不明", "알 수 없음"],
    ["Delete widget", "Delete widget", "Удалить виджет", "Видалити віджет", "Widget löschen", "Supprimer le widget", "Eliminar widget", "Elimina widget", "Excluir widget", "Usuń widżet", "删除小组件", "ウィジェットを削除", "위젯 삭제"],
    ["Size", "Size", "Размер", "Розмір", "Größe", "Taille", "Tamaño", "Dimensioni", "Tamanho", "Rozmiar", "大小", "サイズ", "크기"],
    ["Small", "Small", "Маленький", "Малий", "Klein", "Petite", "Pequeño", "Piccolo", "Pequeno", "Mały", "小", "小", "작게"],
    ["Normal|size", "Normal", "Обычный", "Звичайний", "Normal", "Normale", "Normal", "Normale", "Normal", "Normalny", "中", "標準", "보통"],
    ["Large", "Large", "Большой", "Великий", "Groß", "Grande", "Grande", "Grande", "Grande", "Duży", "大", "大", "크게"],
    ["Extra large", "Extra large", "Очень большой", "Дуже великий", "Sehr groß", "Très grande", "Muy grande", "Molto grande", "Muito grande", "Bardzo duży", "特大", "特大", "아주 크게"],
    ["Huge", "Huge", "Огромный", "Величезний", "Riesig", "Énorme", "Enorme", "Enorme", "Enorme", "Ogromny", "超大", "最大", "최대"],
    ["Background", "Background", "Фон", "Фон", "Hintergrund", "Arrière-plan", "Fondo", "Sfondo", "Fundo", "Tło", "背景", "背景", "배경"],
    ["Frosted glass", "Frosted glass", "Матовое стекло", "Матове скло", "Milchglas", "Verre dépoli", "Vidrio esmerilado", "Vetro smerigliato", "Vidro fosco", "Szkło matowe", "磨砂玻璃", "すりガラス", "반투명 유리"],
    ["Liquid glass", "Liquid glass", "Жидкое стекло", "Рідке скло", "Flüssigglas", "Verre liquide", "Vidrio líquido", "Vetro liquido", "Vidro líquido", "Płynne szkło", "液态玻璃", "リキッドガラス", "리퀴드 글래스"],
    ["Solid", "Solid", "Сплошной", "Суцільний", "Einfarbig", "Uni", "Sólido", "Tinta unita", "Sólido", "Jednolite", "纯色", "単色", "단색"],
    ["Transparent", "Transparent", "Прозрачный", "Прозорий", "Transparent", "Transparent", "Transparente", "Trasparente", "Transparente", "Przezroczyste", "透明", "透明", "투명"],
    ["Transparency", "Transparency", "Прозрачность", "Прозорість", "Transparenz", "Transparence", "Transparencia", "Trasparenza", "Transparência", "Przezroczystość", "透明度", "透明度", "투명도"],
    ["None|transparency", "None", "Нет", "Немає", "Keine", "Aucune", "Ninguna", "Nessuna", "Nenhuma", "Brak", "无", "なし", "없음"],
    ["Low", "Low", "Слабая", "Слабка", "Gering", "Faible", "Baja", "Bassa", "Baixa", "Niska", "低", "低", "낮음"],
    ["Medium", "Medium", "Средняя", "Середня", "Mittel", "Moyenne", "Media", "Media", "Média", "Średnia", "中", "中", "보통"],
    ["High", "High", "Сильная", "Сильна", "Hoch", "Forte", "Alta", "Alta", "Alta", "Wysoka", "高", "高", "높음"],
    ["Blur", "Blur", "Размытие", "Розмиття", "Unschärfe", "Flou", "Desenfoque", "Sfocatura", "Desfoque", "Rozmycie", "模糊", "ぼかし", "흐림"],
    ["None|blur", "None", "Нет", "Немає", "Keine", "Aucun", "Ninguno", "Nessuna", "Nenhum", "Brak", "无", "なし", "없음"],
    ["Low|blur", "Low", "Слабое", "Слабке", "Gering", "Faible", "Bajo", "Bassa", "Baixo", "Niskie", "低", "弱", "약하게"],
    ["Medium|blur", "Medium", "Среднее", "Середнє", "Mittel", "Moyen", "Medio", "Media", "Médio", "Średnie", "中", "中", "보통"],
    ["High|blur", "High", "Сильное", "Сильне", "Stark", "Fort", "Alto", "Alta", "Alto", "Wysokie", "高", "強", "강하게"],
    ["Tint color", "Tint color", "Цвет фона", "Колір фону", "Tönung", "Teinte", "Color de tinte", "Colore tinta", "Cor de tonalidade", "Kolor tła", "着色", "色合い", "색조"],
    ["Choose color…", "Choose color…", "Выбрать цвет…", "Вибрати колір…", "Farbe wählen…", "Choisir une couleur…", "Elegir color…", "Scegli colore…", "Escolher cor…", "Wybierz kolor…", "选择颜色…", "色を選択…", "색 선택…"],
    ["Default", "Default", "По умолчанию", "За замовчуванням", "Standard", "Par défaut", "Predeterminado", "Predefinito", "Padrão", "Domyślne", "默认", "既定", "기본값"],
    ["Corners", "Corners", "Углы", "Кути", "Ecken", "Coins", "Esquinas", "Angoli", "Cantos", "Narożniki", "边角", "角", "모서리"],
    ["Square", "Square", "Прямые", "Прямі", "Eckig", "Carrés", "Rectas", "Squadrati", "Retos", "Proste", "直角", "直角", "각지게"],
    ["Slightly rounded", "Slightly rounded", "Слегка скруглённые", "Трохи заокруглені", "Leicht abgerundet", "Légèrement arrondis", "Ligeramente redondeadas", "Leggermente arrotondati", "Levemente arredondados", "Lekko zaokrąglone", "微圆", "やや丸く", "약간 둥글게"],
    ["Rounded", "Rounded", "Скруглённые", "Заокруглені", "Abgerundet", "Arrondis", "Redondeadas", "Arrotondati", "Arredondados", "Zaokrąglone", "圆润", "丸く", "둥글게"],
    ["Very rounded", "Very rounded", "Сильно скруглённые", "Дуже заокруглені", "Stark abgerundet", "Très arrondis", "Muy redondeadas", "Molto arrotondati", "Muito arredondados", "Mocno zaokrąglone", "很圆", "とても丸く", "아주 둥글게"],
    ["Border", "Border", "Рамка", "Рамка", "Rahmen", "Bordure", "Borde", "Bordo", "Borda", "Obramowanie", "边框", "枠線", "테두리"],
    ["Shadow", "Shadow", "Тень", "Тінь", "Schatten", "Ombre", "Sombra", "Ombra", "Sombra", "Cień", "阴影", "影", "그림자"],
    ["Show", "Show", "Показать", "Показати", "Anzeigen", "Afficher", "Mostrar", "Mostra", "Mostrar", "Pokaż", "显示", "表示", "표시"],
    ["Hide", "Hide", "Скрыть", "Приховати", "Ausblenden", "Masquer", "Ocultar", "Nascondi", "Ocultar", "Ukryj", "隐藏", "非表示", "숨기기"],
    ["Theme", "Theme", "Тема", "Тема", "Design", "Thème", "Tema", "Tema", "Tema", "Motyw", "主题", "テーマ", "테마"],
    ["Match system", "Match system", "Как в системе", "Як у системі", "Wie System", "Comme le système", "Como el sistema", "Come il sistema", "Como o sistema", "Jak w systemie", "跟随系统", "システムに合わせる", "시스템 설정"],
    ["Light|theme", "Light", "Светлая", "Світла", "Hell", "Clair", "Claro", "Chiaro", "Claro", "Jasny", "浅色", "ライト", "라이트"],
    ["Dark", "Dark", "Тёмная", "Темна", "Dunkel", "Sombre", "Oscuro", "Scuro", "Escuro", "Ciemny", "深色", "ダーク", "다크"],
    ["Accent color", "Accent color", "Цвет акцента", "Колір акценту", "Akzentfarbe", "Couleur d'accentuation", "Color de énfasis", "Colore principale", "Cor de destaque", "Kolor akcentu", "强调色", "アクセントカラー", "강조 색"],
    ["Text color", "Text color", "Цвет текста", "Колір тексту", "Textfarbe", "Couleur du texte", "Color del texto", "Colore testo", "Cor do texto", "Kolor tekstu", "文字颜色", "文字の色", "텍스트 색"],
    ["Font", "Font", "Шрифт", "Шрифт", "Schriftart", "Police", "Fuente", "Carattere", "Fonte", "Czcionka", "字体", "フォント", "글꼴"],
    ["More fonts…", "More fonts…", "Другие шрифты…", "Інші шрифти…", "Weitere Schriftarten…", "Plus de polices…", "Más fuentes…", "Altri caratteri…", "Mais fontes…", "Więcej czcionek…", "更多字体…", "その他のフォント…", "다른 글꼴…"],
    ["Weight", "Weight", "Насыщенность", "Насиченість", "Stärke", "Graisse", "Grosor", "Spessore", "Espessura", "Grubość", "粗细", "太さ", "굵기"],
    ["Light|weight", "Light", "Тонкая", "Тонка", "Dünn", "Maigre", "Fina", "Sottile", "Fina", "Cienka", "细", "細い", "가늘게"],
    ["Regular", "Regular", "Обычная", "Звичайна", "Normal", "Normale", "Normal", "Normale", "Normal", "Normalna", "常规", "標準", "보통"],
    ["Semibold", "Semibold", "Полужирная", "Напівжирна", "Halbfett", "Demi-gras", "Seminegrita", "Semigrassetto", "Seminegrito", "Półgruba", "中粗", "やや太い", "약간 굵게"],
    ["Bold", "Bold", "Жирная", "Жирна", "Fett", "Gras", "Negrita", "Grassetto", "Negrito", "Gruba", "粗", "太い", "굵게"],
    ["Black", "Black", "Сверхжирная", "Наджирна", "Extrafett", "Extra-gras", "Extranegrita", "Nerissimo", "Extranegrito", "Bardzo gruba", "特粗", "極太", "아주 굵게"],
    ["Width", "Width", "Ширина", "Ширина", "Breite", "Largeur", "Anchura", "Larghezza", "Largura", "Szerokość", "宽度", "幅", "너비"],
    ["Condensed", "Condensed", "Узкая", "Вузька", "Schmal", "Étroite", "Estrecha", "Stretta", "Estreita", "Wąska", "窄", "狭い", "좁게"],
    ["Semi-condensed", "Semi-condensed", "Полуузкая", "Напіввузька", "Halbschmal", "Semi-étroite", "Semiestrecha", "Semi-stretta", "Semiestreita", "Półwąska", "较窄", "やや狭い", "약간 좁게"],
    ["Normal|width", "Normal", "Обычная", "Звичайна", "Normal", "Normale", "Normal", "Normale", "Normal", "Normalna", "标准", "標準", "보통"],
    ["Wide", "Wide", "Широкая", "Широка", "Breit", "Large", "Ancha", "Larga", "Larga", "Szeroka", "宽", "広い", "넓게"],
    ["Reset to defaults", "Reset to defaults", "Сбросить настройки", "Скинути налаштування", "Zurücksetzen", "Réinitialiser", "Restablecer", "Ripristina", "Redefinir", "Przywróć domyślne", "恢复默认", "既定に戻す", "기본값으로 재설정"],
    // Calmdesk: gallery, tray menu, installer.
    ["Widgets", "Widgets", "Виджеты", "Віджети", "Widgets", "Widgets", "Widgets", "Widget", "Widgets", "Widżety", "小组件", "ウィジェット", "위젯"],
    ["Drag a widget to your desktop, or click to add it.", "Drag a widget to your desktop, or click to add it.", "Перетащите виджет на рабочий стол или нажмите, чтобы добавить.", "Перетягніть віджет на робочий стіл або натисніть, щоб додати.", "Zieh ein Widget auf den Desktop oder klicke, um es hinzuzufügen.", "Faites glisser un widget sur le bureau ou cliquez pour l’ajouter.", "Arrastra un widget al escritorio o haz clic para añadirlo.", "Trascina un widget sul desktop o fai clic per aggiungerlo.", "Arraste um widget para a área de trabalho ou clique para adicioná-lo.", "Przeciągnij widżet na pulpit lub kliknij, aby go dodać.", "将小组件拖到桌面，或点按以添加。", "ウィジェットをデスクトップにドラッグするか、クリックして追加します。", "위젯을 바탕 화면으로 드래그하거나 클릭하여 추가하세요."],
    ["All widgets", "All widgets", "Все виджеты", "Усі віджети", "Alle Widgets", "Tous les widgets", "Todos los widgets", "Tutti i widget", "Todos os widgets", "Wszystkie widżety", "所有小组件", "すべてのウィジェット", "모든 위젯"],
    ["Time", "Time", "Время", "Час", "Zeit", "Heure", "Hora", "Ora", "Hora", "Czas", "时间", "時計", "시간"],
    ["Media", "Media", "Медиа", "Медіа", "Medien", "Médias", "Multimedia", "Multimedia", "Mídia", "Multimedia", "媒体", "メディア", "미디어"],
    ["Other", "Other", "Другое", "Інше", "Sonstiges", "Autres", "Otros", "Altro", "Outros", "Inne", "其他", "その他", "기타"],
    ["Built-in", "Built-in", "Встроенный", "Вбудований", "Integriert", "Intégré", "Integrado", "Integrato", "Integrado", "Wbudowany", "内置", "内蔵", "기본 제공"],
    ["Glass tint", "Glass tint", "Оттенок стекла", "Відтінок скла", "Glastönung", "Teinte du verre", "Tinte del cristal", "Tinta del vetro", "Tonalidade do vidro", "Odcień szkła", "玻璃色调", "ガラスの色合い", "유리 색조"],
    ["None|tint", "None", "Нет", "Немає", "Keine", "Aucune", "Ninguno", "Nessuna", "Nenhuma", "Brak", "无", "なし", "없음"],
    ["Small|tint", "Small", "Слабый", "Слабкий", "Leicht", "Légère", "Leve", "Leggera", "Leve", "Lekki", "轻微", "弱", "약하게"],
    ["Medium|tint", "Medium", "Средний", "Середній", "Mittel", "Moyenne", "Media", "Media", "Média", "Średni", "中等", "中", "보통"],
    ["Strong|tint", "Strong", "Сильный", "Сильний", "Stark", "Forte", "Fuerte", "Forte", "Forte", "Mocny", "强", "強", "강하게"],
    ["Search", "Search", "Поиск", "Пошук", "Suchen", "Rechercher", "Buscar", "Cerca", "Pesquisar", "Szukaj", "搜索", "検索", "검색"],
    ["No widgets found", "No widgets found", "Виджеты не найдены", "Віджетів не знайдено", "Keine Widgets gefunden", "Aucun widget trouvé", "No se encontraron widgets", "Nessun widget trovato", "Nenhum widget encontrado", "Nie znaleziono widżetów", "未找到小组件", "ウィジェットが見つかりません", "위젯을 찾을 수 없음"],
    ["System", "System", "Система", "Система", "System", "Système", "Sistema", "Sistema", "Sistema", "System", "系统", "システム", "시스템"],
    ["Photos", "Photos", "Фото", "Фото", "Fotos", "Photos", "Fotos", "Foto", "Fotos", "Zdjęcia", "照片", "写真", "사진"],
    ["Productivity", "Productivity", "Продуктивность", "Продуктивність", "Produktivität", "Productivité", "Productividad", "Produttività", "Produtividade", "Produktywność", "效率", "仕事効率化", "생산성"],
    ["Weather", "Weather", "Погода", "Погода", "Wetter", "Météo", "Tiempo", "Meteo", "Clima", "Pogoda", "天气", "天気", "날씨"],
    ["Background color", "Background color", "Цвет фона", "Колір тла", "Hintergrundfarbe", "Couleur de fond", "Color de fondo", "Colore di sfondo", "Cor de fundo", "Kolor tła", "背景颜色", "背景色", "배경 색상"],
    ["Graphite", "Graphite", "Графит", "Графіт", "Graphit", "Graphite", "Grafito", "Grafite", "Grafite", "Grafit", "石墨", "グラファイト", "그래파이트"],
    ["Midnight", "Midnight", "Полночь", "Опівнічний", "Mitternacht", "Minuit", "Medianoche", "Mezzanotte", "Meia-noite", "Północ", "午夜蓝", "ミッドナイト", "미드나이트"],
    ["Ocean", "Ocean", "Океан", "Океан", "Ozean", "Océan", "Océano", "Oceano", "Oceano", "Ocean", "海洋", "オーシャン", "오션"],
    ["Teal", "Teal", "Бирюза", "Бірюза", "Petrol", "Sarcelle", "Verde azulado", "Verde acqua", "Azul-petróleo", "Morski", "青色", "ティール", "청록"],
    ["Forest", "Forest", "Лес", "Ліс", "Wald", "Forêt", "Bosque", "Foresta", "Floresta", "Las", "森林", "フォレスト", "포레스트"],
    ["Plum", "Plum", "Слива", "Слива", "Pflaume", "Prune", "Ciruela", "Prugna", "Ameixa", "Śliwka", "梅紫", "プラム", "플럼"],
    ["Berry", "Berry", "Ягода", "Ягода", "Beere", "Baie", "Baya", "Bacca", "Frutos vermelhos", "Jagoda", "莓红", "ベリー", "베리"],
    ["Coral", "Coral", "Коралл", "Корал", "Koralle", "Corail", "Coral", "Corallo", "Coral", "Koral", "珊瑚", "コーラル", "코랄"],
    ["Amber", "Amber", "Янтарь", "Бурштин", "Bernstein", "Ambre", "Ámbar", "Ambra", "Âmbar", "Bursztyn", "琥珀", "アンバー", "앰버"],
    ["Sand", "Sand", "Песок", "Пісок", "Sand", "Sable", "Arena", "Sabbia", "Areia", "Piasek", "沙色", "サンド", "샌드"],
    ["Mint", "Mint", "Мята", "М'ята", "Minze", "Menthe", "Menta", "Menta", "Menta", "Mięta", "薄荷", "ミント", "민트"],
    ["Sky", "Sky", "Небо", "Небо", "Himmel", "Ciel", "Cielo", "Cielo", "Céu", "Niebo", "天空", "スカイ", "스카이"],
    ["Lavender", "Lavender", "Лаванда", "Лаванда", "Lavendel", "Lavande", "Lavanda", "Lavanda", "Lavanda", "Lawenda", "薰衣草", "ラベンダー", "라벤더"],
    ["Blush", "Blush", "Румянец", "Рум'янець", "Rosé", "Rose poudré", "Rubor", "Cipria", "Rosé", "Róż", "腮红粉", "ブラッシュ", "블러시"],
    ["Rounded|font", "Rounded", "Округлые", "Округлі", "Abgerundet", "Arrondies", "Redondeadas", "Arrotondati", "Arredondadas", "Zaokrąglone", "圆润", "丸ゴシック", "둥근"],
    ["Modern|font", "Modern", "Современные", "Сучасні", "Modern", "Modernes", "Modernas", "Moderni", "Modernas", "Nowoczesne", "现代", "モダン", "모던"],
    ["Sharp|font", "Sharp", "Строгие", "Строгі", "Kantig", "Anguleuses", "Angulosas", "Spigolosi", "Angulosas", "Ostre", "硬朗", "シャープ", "샤프"],
    ["Techno|font", "Techno", "Техно", "Техно", "Techno", "Techno", "Tecno", "Techno", "Techno", "Techno", "科技", "テクノ", "테크노"],
    ["Monospace|font", "Monospace", "Моноширинные", "Моноширинні", "Monospace", "Chasse fixe", "Monoespaciadas", "Monospazio", "Monoespaçadas", "O stałej szerokości", "等宽", "等幅", "고정폭"],
    ["Choose photo…", "Choose photo…", "Выбрать фото…", "Вибрати фото…", "Foto auswählen…", "Choisir une photo…", "Elegir foto…", "Scegli foto…", "Escolher foto…", "Wybierz zdjęcie…", "选择照片…", "写真を選択…", "사진 선택…"],
    ["Choose folder…", "Choose folder…", "Выбрать папку…", "Вибрати папку…", "Ordner auswählen…", "Choisir un dossier…", "Elegir carpeta…", "Scegli cartella…", "Escolher pasta…", "Wybierz folder…", "选择文件夹…", "フォルダーを選択…", "폴더 선택…"],
    ["None|path", "None", "Нет", "Немає", "Keine", "Aucun", "Ninguno", "Nessuna", "Nenhuma", "Brak", "无", "なし", "없음"],
    ["Pictures|files", "Pictures", "Изображения", "Зображення", "Bilder", "Images", "Imágenes", "Immagini", "Imagens", "Obrazy", "图片", "画像", "사진"],
    ["Widget gallery", "Widget gallery", "Галерея виджетов", "Галерея віджетів", "Widget-Galerie", "Galerie de widgets", "Galería de widgets", "Galleria widget", "Galeria de widgets", "Galeria widżetów", "小组件库", "ウィジェット ギャラリー", "위젯 갤러리"],
    ["Show widgets", "Show widgets", "Показать виджеты", "Показати віджети", "Widgets einblenden", "Afficher les widgets", "Mostrar widgets", "Mostra widget", "Mostrar widgets", "Pokaż widżety", "显示小组件", "ウィジェットを表示", "위젯 표시"],
    ["Hide widgets", "Hide widgets", "Скрыть виджеты", "Сховати віджети", "Widgets ausblenden", "Masquer les widgets", "Ocultar widgets", "Nascondi widget", "Ocultar widgets", "Ukryj widżety", "隐藏小组件", "ウィジェットを非表示", "위젯 숨기기"],
    ["Remove all widgets", "Remove all widgets", "Удалить все виджеты", "Видалити всі віджети", "Alle Widgets entfernen", "Supprimer tous les widgets", "Quitar todos los widgets", "Rimuovi tutti i widget", "Remover todos os widgets", "Usuń wszystkie widżety", "移除所有小组件", "すべてのウィジェットを削除", "모든 위젯 제거"],
    ["Start with Windows", "Start with Windows", "Запускать вместе с Windows", "Запускати разом з Windows", "Mit Windows starten", "Lancer au démarrage de Windows", "Iniciar con Windows", "Avvia con Windows", "Iniciar com o Windows", "Uruchamiaj z systemem Windows", "随 Windows 启动", "Windows と同時に起動", "Windows 시작 시 실행"],
    ["Settings", "Settings", "Настройки", "Налаштування", "Einstellungen", "Paramètres", "Configuración", "Impostazioni", "Configurações", "Ustawienia", "设置", "設定", "설정"],
    ["Hide widgets during fullscreen apps", "Hide widgets during fullscreen apps", "Скрывать виджеты в полноэкранных приложениях", "Ховати віджети в повноекранних застосунках", "Widgets bei Vollbild-Apps ausblenden", "Masquer les widgets pendant les apps en plein écran", "Ocultar widgets con apps en pantalla completa", "Nascondi i widget con app a schermo intero", "Ocultar widgets com apps em tela cheia", "Ukrywaj widżety w aplikacjach pełnoekranowych", "全屏应用时隐藏小组件", "全画面アプリの実行中はウィジェットを隠す", "전체 화면 앱에서 위젯 숨기기"],
    ["Language", "Language", "Язык", "Мова", "Sprache", "Langue", "Idioma", "Lingua", "Idioma", "Język", "语言", "言語", "언어"],
    ["System default", "System default", "Как в системе", "Як у системі", "Systemstandard", "Langue du système", "Predeterminado del sistema", "Predefinita di sistema", "Padrão do sistema", "Domyślny systemowy", "系统默认", "システムの既定", "시스템 기본값"],
    ["Export widgets", "Export widgets", "Экспорт виджетов", "Експорт віджетів", "Widgets exportieren", "Exporter les widgets", "Exportar widgets", "Esporta widget", "Exportar widgets", "Eksportuj widżety", "导出小组件", "ウィジェットをエクスポート", "위젯 내보내기"],
    ["Import widgets", "Import widgets", "Импорт виджетов", "Імпорт віджетів", "Widgets importieren", "Importer les widgets", "Importar widgets", "Importa widget", "Importar widgets", "Importuj widżety", "导入小组件", "ウィジェットをインポート", "위젯 가져오기"],
    ["Open log folder", "Open log folder", "Открыть папку журналов", "Відкрити теку журналів", "Protokollordner öffnen", "Ouvrir le dossier des journaux", "Abrir carpeta de registros", "Apri cartella dei log", "Abrir pasta de logs", "Otwórz folder dzienników", "打开日志文件夹", "ログ フォルダーを開く", "로그 폴더 열기"],
    ["Check for updates", "Check for updates", "Проверить обновления", "Перевірити оновлення", "Nach Updates suchen", "Rechercher des mises à jour", "Buscar actualizaciones", "Controlla aggiornamenti", "Verificar atualizações", "Sprawdź aktualizacje", "检查更新", "更新を確認", "업데이트 확인"],
    ["Update available", "Update available", "Доступно обновление", "Доступне оновлення", "Update verfügbar", "Mise à jour disponible", "Actualización disponible", "Aggiornamento disponibile", "Atualização disponível", "Dostępna aktualizacja", "有可用更新", "更新があります", "업데이트 있음"],
    ["Calmdesk is up to date.", "Calmdesk is up to date.", "У вас последняя версия Calmdesk.", "У вас остання версія Calmdesk.", "Calmdesk ist auf dem neuesten Stand.", "Calmdesk est à jour.", "Calmdesk está actualizado.", "Calmdesk è aggiornato.", "O Calmdesk está atualizado.", "Calmdesk jest aktualny.", "Calmdesk 已是最新版本。", "Calmdesk は最新です。", "Calmdesk가 최신 버전입니다."],
    ["A new version of Calmdesk is out. Open its page?", "A new version of Calmdesk is out. Open its page?", "Вышла новая версия Calmdesk. Открыть её страницу?", "Вийшла нова версія Calmdesk. Відкрити її сторінку?", "Eine neue Version von Calmdesk ist da. Seite öffnen?", "Une nouvelle version de Calmdesk est disponible. Ouvrir sa page ?", "Hay una nueva versión de Calmdesk. ¿Abrir su página?", "È uscita una nuova versione di Calmdesk. Aprire la pagina?", "Saiu uma nova versão do Calmdesk. Abrir a página?", "Jest nowa wersja Calmdesk. Otworzyć jej stronę?", "Calmdesk 有新版本。打开其页面？", "Calmdesk の新しいバージョンがあります。ページを開きますか?", "Calmdesk 새 버전이 나왔습니다. 페이지를 열까요?"],
    ["Could not check for updates.", "Could not check for updates.", "Не удалось проверить обновления.", "Не вдалося перевірити оновлення.", "Suche nach Updates fehlgeschlagen.", "Impossible de rechercher des mises à jour.", "No se pudieron buscar actualizaciones.", "Impossibile controllare gli aggiornamenti.", "Não foi possível verificar atualizações.", "Nie udało się sprawdzić aktualizacji.", "无法检查更新。", "更新を確認できませんでした。", "업데이트를 확인할 수 없습니다."],
    ["No backup found:", "No backup found:", "Резервная копия не найдена:", "Резервну копію не знайдено:", "Keine Sicherung gefunden:", "Aucune sauvegarde trouvée :", "No se encontró ninguna copia:", "Nessun backup trovato:", "Nenhum backup encontrado:", "Nie znaleziono kopii:", "未找到备份：", "バックアップが見つかりません:", "백업을 찾을 수 없음:"],
    ["Quit", "Quit", "Выйти", "Вийти", "Beenden", "Quitter", "Salir", "Esci", "Sair", "Zakończ", "退出", "終了", "종료"],
    ["Install", "Install", "Установить", "Встановити", "Installieren", "Installer", "Instalar", "Installa", "Instalar", "Zainstaluj", "安装", "インストール", "설치"],
    ["Installing…", "Installing…", "Установка…", "Встановлення…", "Wird installiert…", "Installation…", "Instalando…", "Installazione…", "Instalando…", "Instalowanie…", "正在安装…", "インストール中…", "설치 중…"],
    ["Installed", "Installed", "Установлено", "Встановлено", "Installiert", "Installé", "Instalado", "Installato", "Instalado", "Zainstalowano", "已安装", "インストール完了", "설치됨"],
    ["Open", "Open", "Открыть", "Відкрити", "Öffnen", "Ouvrir", "Abrir", "Apri", "Abrir", "Otwórz", "打开", "開く", "열기"],
    ["Desktop widgets for Windows", "Desktop widgets for Windows", "Виджеты рабочего стола для Windows", "Віджети робочого столу для Windows", "Desktop-Widgets für Windows", "Widgets de bureau pour Windows", "Widgets de escritorio para Windows", "Widget del desktop per Windows", "Widgets da área de trabalho para Windows", "Widżety pulpitu dla Windows", "适用于 Windows 的桌面小组件", "Windows 用デスクトップ ウィジェット", "Windows용 바탕 화면 위젯"],
    ["Featured", "Featured", "Рекомендуемые", "Рекомендовані", "Empfohlen", "À la une", "Destacados", "In evidenza", "Destaques", "Polecane", "精选", "おすすめ", "추천"],
    ["What's new in", "What's new in", "Что нового в", "Що нового у", "Neu in", "Nouveautés :", "Novedades:", "Novità:", "Novidades:", "Co nowego w", "新功能：", "新機能：", "새로운 기능:"],
    ["Profiles", "Profiles", "Профили", "Профілі", "Profile", "Profils", "Perfiles", "Profili", "Perfis", "Profile", "配置", "プロファイル", "프로필"],
    ["Profile", "Profile", "Профиль", "Профіль", "Profil", "Profil", "Perfil", "Profilo", "Perfil", "Profil", "配置", "プロファイル", "프로필"],
    ["Save as new profile", "Save as new profile", "Сохранить как новый профиль", "Зберегти як новий профіль", "Als neues Profil speichern", "Enregistrer comme nouveau profil", "Guardar como perfil nuevo", "Salva come nuovo profilo", "Salvar como novo perfil", "Zapisz jako nowy profil", "另存为新配置", "新しいプロファイルとして保存", "새 프로필로 저장"],
    ["Open profiles folder", "Open profiles folder", "Открыть папку профилей", "Відкрити теку профілів", "Profilordner öffnen", "Ouvrir le dossier des profils", "Abrir la carpeta de perfiles", "Apri la cartella dei profili", "Abrir a pasta de perfis", "Otwórz folder profili", "打开配置文件夹", "プロファイルのフォルダーを開く", "프로필 폴더 열기"],
    ["Other apps", "Other apps", "Другие приложения", "Інші програми", "Andere Apps", "Autres applications", "Otras aplicaciones", "Altre app", "Outros aplicativos", "Inne aplikacje", "其他应用", "その他のアプリ", "다른 앱"],
    ["Community", "Community", "Сообщество", "Спільнота", "Community", "Communauté", "Comunidad", "Community", "Comunidade", "Społeczność", "社区", "コミュニティ", "커뮤니티"],
    ["Install this widget?", "Install this widget?", "Установить этот виджет?", "Встановити цей віджет?", "Dieses Widget installieren?", "Installer ce widget ?", "¿Instalar este widget?", "Installare questo widget?", "Instalar este widget?", "Zainstalować ten widżet?", "安装此小组件？", "このウィジェットをインストールしますか？", "이 위젯을 설치할까요?"],
    ["Update this widget? The installed version is replaced.", "Update this widget? The installed version is replaced.", "Обновить этот виджет? Установленная версия будет заменена.", "Оновити цей віджет? Встановлену версію буде замінено.", "Dieses Widget aktualisieren? Die installierte Version wird ersetzt.", "Mettre à jour ce widget ? La version installée sera remplacée.", "¿Actualizar este widget? Se reemplazará la versión instalada.", "Aggiornare questo widget? La versione installata verrà sostituita.", "Atualizar este widget? A versão instalada será substituída.", "Zaktualizować ten widżet? Zainstalowana wersja zostanie zastąpiona.", "更新此小组件？已安装的版本将被替换。", "このウィジェットを更新しますか？インストール済みのバージョンは置き換えられます。", "이 위젯을 업데이트할까요? 설치된 버전이 바뀝니다."],
    ["Installation failed:", "Installation failed:", "Не удалось установить:", "Не вдалося встановити:", "Installation fehlgeschlagen:", "Échec de l’installation :", "Error de instalación:", "Installazione non riuscita:", "Falha na instalação:", "Instalacja nie powiodła się:", "安装失败：", "インストールに失敗しました:", "설치 실패:"],
    ["Uninstall Calmdesk and remove its widgets?", "Uninstall Calmdesk and remove its widgets?", "Удалить Calmdesk и его виджеты?", "Видалити Calmdesk і його віджети?", "Calmdesk und seine Widgets deinstallieren?", "Désinstaller Calmdesk et ses widgets ?", "¿Desinstalar Calmdesk y sus widgets?", "Disinstallare Calmdesk e i suoi widget?", "Desinstalar o Calmdesk e seus widgets?", "Odinstalować Calmdesk i jego widżety?", "要卸载 Calmdesk 及其小组件吗？", "Calmdesk とそのウィジェットをアンインストールしますか?", "Calmdesk와 위젯을 제거할까요?"],
    // Texts the app puts in widgets: world clock days and cities, gallery samples.
    ["Today", "Today", "Сегодня", "Сьогодні", "Heute", "Aujourd’hui", "Hoy", "Oggi", "Hoje", "Dziś", "今天", "今日", "오늘"],
    ["Yesterday", "Yesterday", "Вчера", "Учора", "Gestern", "Hier", "Ayer", "Ieri", "Ontem", "Wczoraj", "昨天", "昨日", "어제"],
    ["Tomorrow", "Tomorrow", "Завтра", "Завтра", "Morgen", "Demain", "Mañana", "Domani", "Amanhã", "Jutro", "明天", "明日", "내일"],
    ["Los Angeles", "Los Angeles", "Лос-Анджелес", "Лос-Анджелес", "Los Angeles", "Los Angeles", "Los Ángeles", "Los Angeles", "Los Angeles", "Los Angeles", "洛杉矶", "ロサンゼルス", "로스앤젤레스"],
    ["New York", "New York", "Нью-Йорк", "Нью-Йорк", "New York", "New York", "Nueva York", "New York", "Nova York", "Nowy Jork", "纽约", "ニューヨーク", "뉴욕"],
    ["São Paulo", "São Paulo", "Сан-Паулу", "Сан-Паулу", "São Paulo", "São Paulo", "São Paulo", "San Paolo", "São Paulo", "São Paulo", "圣保罗", "サンパウロ", "상파울루"],
    ["London", "London", "Лондон", "Лондон", "London", "Londres", "Londres", "Londra", "Londres", "Londyn", "伦敦", "ロンドン", "런던"],
    ["Paris", "Paris", "Париж", "Париж", "Paris", "Paris", "París", "Parigi", "Paris", "Paryż", "巴黎", "パリ", "파리"],
    ["Berlin", "Berlin", "Берлин", "Берлін", "Berlin", "Berlin", "Berlín", "Berlino", "Berlim", "Berlin", "柏林", "ベルリン", "베를린"],
    ["Kyiv", "Kyiv", "Киев", "Київ", "Kyjiw", "Kyiv", "Kiev", "Kiev", "Kiev", "Kijów", "基辅", "キーウ", "키이우"],
    ["Istanbul", "Istanbul", "Стамбул", "Стамбул", "Istanbul", "Istanbul", "Estambul", "Istanbul", "Istambul", "Stambuł", "伊斯坦布尔", "イスタンブール", "이스탄불"],
    ["Moscow", "Moscow", "Москва", "Москва", "Moskau", "Moscou", "Moscú", "Mosca", "Moscou", "Moskwa", "莫斯科", "モスクワ", "모스크바"],
    ["Dubai", "Dubai", "Дубай", "Дубай", "Dubai", "Dubaï", "Dubái", "Dubai", "Dubai", "Dubaj", "迪拜", "ドバイ", "두바이"],
    ["New Delhi", "New Delhi", "Нью-Дели", "Нью-Делі", "Neu-Delhi", "New Delhi", "Nueva Delhi", "Nuova Delhi", "Nova Délhi", "Nowe Delhi", "新德里", "ニューデリー", "뉴델리"],
    ["Bangkok", "Bangkok", "Бангкок", "Бангкок", "Bangkok", "Bangkok", "Bangkok", "Bangkok", "Bangkok", "Bangkok", "曼谷", "バンコク", "방콕"],
    ["Beijing", "Beijing", "Пекин", "Пекін", "Peking", "Pékin", "Pekín", "Pechino", "Pequim", "Pekin", "北京", "北京", "베이징"],
    ["Tokyo", "Tokyo", "Токио", "Токіо", "Tokio", "Tokyo", "Tokio", "Tokyo", "Tóquio", "Tokio", "东京", "東京", "도쿄"],
    ["Sydney", "Sydney", "Сидней", "Сідней", "Sydney", "Sydney", "Sídney", "Sydney", "Sydney", "Sydney", "悉尼", "シドニー", "시드니"],
    ["Pick up flowers", "Pick up flowers", "Забрать цветы", "Забрати квіти", "Blumen abholen", "Acheter des fleurs", "Recoger flores", "Prendere i fiori", "Buscar flores", "Odebrać kwiaty", "取花", "花を受け取る", "꽃 찾아오기"],
    ["Call Anna at six", "Call Anna at six", "Позвонить Анне в шесть", "Зателефонувати Анні о шостій", "Anna um sechs anrufen", "Appeler Anna à six heures", "Llamar a Ana a las seis", "Chiamare Anna alle sei", "Ligar para a Ana às seis", "Zadzwonić do Anny o szóstej", "六点给安娜打电话", "6時にアンナに電話", "6시에 안나에게 전화"],
    ["Book tickets for Friday", "Book tickets for Friday", "Купить билеты на пятницу", "Купити квитки на пʼятницю", "Tickets für Freitag buchen", "Réserver les billets pour vendredi", "Reservar entradas para el viernes", "Prenotare i biglietti per venerdì", "Reservar ingressos para sexta", "Zarezerwować bilety na piątek", "订周五的票", "金曜日のチケットを予約", "금요일 티켓 예매"],
    ["Buy groceries", "Buy groceries", "Купить продукты", "Купити продукти", "Lebensmittel einkaufen", "Faire les courses", "Comprar comida", "Fare la spesa", "Comprar mantimentos", "Zrobić zakupy", "买菜", "食料品を買う", "장보기"],
    ["Reply to emails", "Reply to emails", "Ответить на письма", "Відповісти на листи", "E-Mails beantworten", "Répondre aux e-mails", "Responder correos", "Rispondere alle email", "Responder e-mails", "Odpisać na maile", "回复邮件", "メールに返信", "이메일 답장"],
    ["Water the plants", "Water the plants", "Полить цветы", "Полити квіти", "Pflanzen gießen", "Arroser les plantes", "Regar las plantas", "Annaffiare le piante", "Regar as plantas", "Podlać kwiaty", "浇花", "植物に水やり", "화분에 물 주기"],
    ["Go for a run", "Go for a run", "Пробежка", "Пробіжка", "Joggen gehen", "Aller courir", "Salir a correr", "Andare a correre", "Correr", "Pobiegać", "去跑步", "ランニング", "달리기"],
    ["Saint Petersburg", "Saint Petersburg", "Санкт-Петербург", "Санкт-Петербург", "Sankt Petersburg", "Saint-Pétersbourg", "San Petersburgo", "San Pietroburgo", "São Petersburgo", "Petersburg", "圣彼得堡", "サンクトペテルブルク", "상트페테르부르크"],
    ["Yekaterinburg", "Yekaterinburg", "Екатеринбург", "Єкатеринбург", "Jekaterinburg", "Iekaterinbourg", "Ekaterimburgo", "Ekaterinburg", "Ecaterimburgo", "Jekaterynburg", "叶卡捷琳堡", "エカテリンブルク", "예카테린부르크"],
    ["Novosibirsk", "Novosibirsk", "Новосибирск", "Новосибірськ", "Nowosibirsk", "Novossibirsk", "Novosibirsk", "Novosibirsk", "Novosibirsk", "Nowosybirsk", "新西伯利亚", "ノヴォシビルスク", "노보시비르스크"],
    ["Kazan", "Kazan", "Казань", "Казань", "Kasan", "Kazan", "Kazán", "Kazan", "Kazan", "Kazań", "喀山", "カザン", "카잔"],
    ["Minsk", "Minsk", "Минск", "Мінськ", "Minsk", "Minsk", "Minsk", "Minsk", "Minsk", "Mińsk", "明斯克", "ミンスク", "민스크"],
    ["Almaty", "Almaty", "Алматы", "Алмати", "Almaty", "Almaty", "Almaty", "Almaty", "Almaty", "Ałmaty", "阿拉木图", "アルマトイ", "알마티"],
    ["Kharkiv", "Kharkiv", "Харьков", "Харків", "Charkiw", "Kharkiv", "Járkov", "Charkiv", "Carcóvia", "Charków", "哈尔科夫", "ハルキウ", "하르키우"],
    ["Odesa", "Odesa", "Одесса", "Одеса", "Odessa", "Odessa", "Odesa", "Odessa", "Odessa", "Odessa", "敖德萨", "オデーサ", "오데사"],
    ["Lviv", "Lviv", "Львов", "Львів", "Lwiw", "Lviv", "Leópolis", "Leopoli", "Lviv", "Lwów", "利沃夫", "リヴィウ", "리비우"],
    ["Warsaw", "Warsaw", "Варшава", "Варшава", "Warschau", "Varsovie", "Varsovia", "Varsavia", "Varsóvia", "Warszawa", "华沙", "ワルシャワ", "바르샤바"],
    ["Prague", "Prague", "Прага", "Прага", "Prag", "Prague", "Praga", "Praga", "Praga", "Praga", "布拉格", "プラハ", "프라하"],
    ["Vienna", "Vienna", "Вена", "Відень", "Wien", "Vienne", "Viena", "Vienna", "Viena", "Wiedeń", "维也纳", "ウィーン", "빈"],
    ["Rome", "Rome", "Рим", "Рим", "Rom", "Rome", "Roma", "Roma", "Roma", "Rzym", "罗马", "ローマ", "로마"],
    ["Madrid", "Madrid", "Мадрид", "Мадрид", "Madrid", "Madrid", "Madrid", "Madrid", "Madri", "Madryt", "马德里", "マドリード", "마드리드"],
    ["Amsterdam", "Amsterdam", "Амстердам", "Амстердам", "Amsterdam", "Amsterdam", "Ámsterdam", "Amsterdam", "Amsterdã", "Amsterdam", "阿姆斯特丹", "アムステルダム", "암스테르담"],
    ["Chicago", "Chicago", "Чикаго", "Чикаго", "Chicago", "Chicago", "Chicago", "Chicago", "Chicago", "Chicago", "芝加哥", "シカゴ", "시카고"],
    ["Toronto", "Toronto", "Торонто", "Торонто", "Toronto", "Toronto", "Toronto", "Toronto", "Toronto", "Toronto", "多伦多", "トロント", "토론토"],
    ["Seoul", "Seoul", "Сеул", "Сеул", "Seoul", "Séoul", "Seúl", "Seul", "Seul", "Seul", "首尔", "ソウル", "서울"],
    ["Singapore", "Singapore", "Сингапур", "Сінгапур", "Singapur", "Singapour", "Singapur", "Singapore", "Singapura", "Singapur", "新加坡", "シンガポール", "싱가포르"],
    // Groups of long option lists.
    ["Americas", "Americas", "Америка", "Америка", "Amerika", "Amériques", "América", "Americhe", "Américas", "Ameryka", "美洲", "南北アメリカ", "아메리카"],
    ["Europe", "Europe", "Европа", "Європа", "Europa", "Europe", "Europa", "Europa", "Europa", "Europa", "欧洲", "ヨーロッパ", "유럽"],
    ["Eastern Europe", "Eastern Europe", "Восточная Европа", "Східна Європа", "Osteuropa", "Europe de l’Est", "Europa del Este", "Europa orientale", "Europa Oriental", "Europa Wschodnia", "东欧", "東ヨーロッパ", "동유럽"],
    ["Asia & Pacific", "Asia & Pacific", "Азия и Океания", "Азія та Океанія", "Asien und Pazifik", "Asie-Pacifique", "Asia y Pacífico", "Asia e Pacifico", "Ásia e Pacífico", "Azja i Pacyfik", "亚太地区", "アジア太平洋", "아시아 태평양"],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_unique_and_english_column() {
        for (i, row) in STRINGS.iter().enumerate() {
            assert_eq!(row[0].split('|').next(), Some(row[1]), "row {i}: English column must match the key");
            assert!(STRINGS[..i].iter().all(|r| r[0] != row[0]), "duplicate key {}", row[0]);
        }
        assert_eq!(t("no such key"), "no such key");
    }
}
