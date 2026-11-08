//! UI translations.
//!
//! English is the source language. [`tr`] maps an English `&'static str`
//! to the current language's translation, falling back to the English
//! source. Strings with parameters are formatted at the call site from
//! translated fragments.
//!
//! Every user-facing `label()` method and the command registry's labels
//! and descriptions route through [`tr`]; the catalog below is the single
//! source of translations. Spanish (`es`) and Simplified Chinese
//! (`zh-Hans`) are provided; other languages can extend [`Language`] and
//! the `ENTRIES` table.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// The UI language. English is the default and the source language.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Language {
    #[default]
    En,
    Es,
    ZhHans,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::En, Language::Es, Language::ZhHans];

    /// The language's name for the picker. Kept in the Latin script so it
    /// renders without a CJK font; the CJK UI itself needs a CJK font
    /// (see the translation README note).
    pub fn name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Es => "Español",
            Language::ZhHans => "Chinese (Simplified)",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Language::En as u8);

/// Set the process-wide UI language.
pub fn set(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

/// The process-wide UI language.
pub fn current() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Language::Es,
        2 => Language::ZhHans,
        _ => Language::En,
    }
}

/// Translate an English string. Unknown strings return the English source.
pub fn tr(english: &'static str) -> &'static str {
    match current() {
        Language::En => english,
        Language::Es => es().get(english).copied().unwrap_or(english),
        Language::ZhHans => zh().get(english).copied().unwrap_or(english),
    }
}

fn es() -> &'static std::collections::HashMap<&'static str, &'static str> {
    static ES: OnceLock<std::collections::HashMap<&'static str, &'static str>> = OnceLock::new();
    ES.get_or_init(|| ENTRIES.iter().map(|&(en, es, _)| (en, es)).collect())
}

fn zh() -> &'static std::collections::HashMap<&'static str, &'static str> {
    static ZH: OnceLock<std::collections::HashMap<&'static str, &'static str>> = OnceLock::new();
    ZH.get_or_init(|| ENTRIES.iter().map(|&(en, _, zh)| (en, zh)).collect())
}

/// `(English, Spanish, Simplified Chinese)`.
#[rustfmt::skip]
const ENTRIES: &[(&str, &str, &str)] = &[
    // ---- Categories ----
    ("Capture", "Captura", "采集"),
    ("Signal Navigation", "Navegación de señal", "信号导航"),
    ("Analog Signals", "Señales analógicas", "模拟信号"),
    ("Sessions", "Sesiones", "会话"),
    ("Markers", "Marcadores", "标记"),
    ("Measurements", "Mediciones", "测量"),
    ("Inspector", "Inspector", "检查器"),
    ("Misc", "Varios", "其他"),
    ("Data", "Datos", "数据"),
    ("View History", "Historial de vista", "视图历史"),

    // ---- Command labels ----
    ("Start/Stop Recording", "Iniciar/Detener grabación", "开始/停止录制"),
    ("Repeat Capture", "Repetir captura", "重复采集"),
    ("Save Capture", "Guardar captura", "保存采集"),
    ("Save Capture As…", "Guardar captura como…", "采集另存为…"),
    ("Open Capture…", "Abrir captura…", "打开采集…"),
    ("Open Recent", "Abrir reciente", "打开最近"),
    ("Export Data…", "Exportar datos…", "导出数据…"),
    ("Copy oxrok Command", "Copiar comando oxrok", "复制 oxrok 命令"),
    ("Jump to Time", "Ir a un tiempo", "跳转到时间"),
    ("Previous Edge", "Borde anterior", "上一个边沿"),
    ("Next Edge", "Borde siguiente", "下一个边沿"),
    ("Previous Result", "Resultado anterior", "上一个结果"),
    ("Next Result", "Resultado siguiente", "下一个结果"),
    ("Go to Start", "Ir al inicio", "转到开头"),
    ("Go to End", "Ir al final", "转到结尾"),
    ("X Zoom In", "Acercar (X)", "横向放大"),
    ("X Zoom Out", "Alejar (X)", "横向缩小"),
    ("Full Zoom In", "Acercar al máximo", "放到最大"),
    ("Fit All Data", "Ajustar todos los datos", "适应全部数据"),
    ("Pan Left", "Desplazar a la izquierda", "向左平移"),
    ("Pan Right", "Desplazar a la derecha", "向右平移"),
    ("Pan Page Left", "Página a la izquierda", "向左翻页"),
    ("Pan Page Right", "Página a la derecha", "向右翻页"),
    ("Scroll Rows Up", "Desplazar filas arriba", "向上滚动行"),
    ("Scroll Rows Down", "Desplazar filas abajo", "向下滚动行"),
    ("Center Current Trigger", "Centrar disparo actual", "居中当前触发"),
    ("X Range Zoom", "Zoom de rango X", "横向范围缩放"),
    ("Add Analyzer…", "Añadir analizador…", "添加分析器…"),
    ("Pause Analyzers", "Pausar analizadores", "暂停分析器"),
    ("Y Zoom In", "Acercar (Y)", "纵向放大"),
    ("Y Zoom Out", "Alejar (Y)", "纵向缩小"),
    ("Y Fit", "Ajustar Y", "纵向适应"),
    ("XY Range Zoom", "Zoom de rango XY", "XY 范围缩放"),
    ("Y Range Zoom", "Zoom de rango Y", "纵向范围缩放"),
    ("Y Fit All", "Ajustar Y todo", "全部纵向适应"),
    ("New Session", "Nueva sesión", "新建会话"),
    ("Close Session", "Cerrar sesión", "关闭会话"),
    ("Show Sessions", "Mostrar sesiones", "显示会话"),
    ("Next Session", "Sesión siguiente", "下一个会话"),
    ("Previous Session", "Sesión anterior", "上一个会话"),
    ("Go to Session", "Ir a la sesión", "转到会话"),
    ("Rename Session", "Renombrar sesión", "重命名会话"),
    ("Close Other Sessions", "Cerrar otras sesiones", "关闭其他会话"),
    ("Add Marker", "Añadir marcador", "添加标记"),
    ("Add Timing Markers Pair", "Añadir par de marcadores", "添加标记对"),
    ("Delete Selected Markers", "Eliminar marcadores seleccionados", "删除所选标记"),
    ("Select All Markers", "Seleccionar todos los marcadores", "全选标记"),
    ("Add Range Measurement", "Añadir medición de rango", "添加范围测量"),
    ("Measure Pulse", "Medir pulso", "测量脉冲"),
    ("Delete Selected Range Measurements", "Eliminar mediciones seleccionadas", "删除所选范围测量"),
    ("Select All Range Measurements", "Seleccionar todas las mediciones", "全选范围测量"),
    ("Toggle Inspector", "Alternar inspector", "切换检查器"),
    ("Device tab", "Pestaña Dispositivo", "设备标签页"),
    ("Analyzers tab", "Pestaña Analizadores", "分析器标签页"),
    ("Measurements tab", "Pestaña Mediciones", "测量标签页"),
    ("Markers tab", "Pestaña Marcadores", "标记标签页"),
    ("Notes", "Notas", "备注"),
    ("Detach Inspector", "Separar inspector", "分离检查器"),
    ("Toggle Keyboard Shortcuts Menu", "Alternar atajos de teclado", "切换键盘快捷键"),
    ("Device Connection Help…", "Ayuda de conexión…", "设备连接帮助…"),
    ("About", "Acerca de", "关于"),
    ("Start Onboarding", "Iniciar bienvenida", "开始引导"),
    ("Find in Data Table", "Buscar en la tabla", "在数据表中查找"),
    ("Toggle Traces Color", "Alternar color de trazas", "切换波形颜色"),
    ("Wire Status", "Estado de los pines", "连线状态"),
    ("Trace Width", "Grosor de traza", "波形粗细"),
    ("Theme", "Tema", "主题"),
    ("Open Settings", "Abrir ajustes", "打开设置"),
    ("Increase UI Size", "Aumentar tamaño de la interfaz", "放大界面"),
    ("Decrease UI Size", "Reducir tamaño de la interfaz", "缩小界面"),
    ("Reset UI Size", "Restablecer tamaño de la interfaz", "重置界面大小"),
    ("Open Command Palette", "Abrir paleta de comandos", "打开命令面板"),
    ("Reset Recent Commands List", "Restablecer comandos recientes", "重置最近命令"),
    ("Undo", "Deshacer", "撤销"),
    ("Redo", "Rehacer", "重做"),
    ("Delete Data Off Screen", "Eliminar datos fuera de pantalla", "删除屏幕外数据"),
    ("Delete Data Before the View", "Eliminar datos antes de la vista", "删除视图之前的数据"),
    ("Delete Data After the View", "Eliminar datos después de la vista", "删除视图之后的数据"),
    ("Previous View", "Vista anterior", "上一个视图"),
    ("Next View", "Vista siguiente", "下一个视图"),

    // ---- Command descriptions ----
    ("record from the device, or stop", "grabar desde el dispositivo o detener", "从设备录制，或停止"),
    ("wait for the next trigger each time a capture ends", "esperar el siguiente disparo al terminar una captura", "每次采集结束后等待下一个触发"),
    ("save the capture as a .oxrok file", "guardar la captura como archivo .oxrok", "将采集保存为 .oxrok 文件"),
    ("save the capture to a new file", "guardar la captura en un archivo nuevo", "将采集保存到新文件"),
    ("open .oxrok, .sr and other capture files", "abrir archivos .oxrok, .sr y otros", "打开 .oxrok、.sr 等采集文件"),
    ("reopen a capture file", "volver a abrir un archivo de captura", "重新打开采集文件"),
    ("write CSV, binary, VCD or sigrok files", "escribir CSV, binario, VCD o sigrok", "写出 CSV、二进制、VCD 或 sigrok 文件"),
    ("the command line for the current capture settings", "la línea de comandos de la configuración actual", "当前采集设置的命令行"),
    ("center the view on a time", "centrar la vista en un tiempo", "将视图居中到某个时间"),
    ("the edge before the view center (hovered row)", "el borde anterior al centro (fila señalada)", "视图中心之前的边沿（悬停行）"),
    ("the edge after the view center (hovered row)", "el borde posterior al centro (fila señalada)", "视图中心之后的边沿（悬停行）"),
    ("the decoded result before (hovered row)", "el resultado decodificado anterior (fila señalada)", "前一个解码结果（悬停行）"),
    ("the decoded result after (hovered row)", "el resultado decodificado posterior (fila señalada)", "后一个解码结果（悬停行）"),
    ("show the first data", "mostrar los primeros datos", "显示最开始的数据"),
    ("show the newest data", "mostrar los datos más recientes", "显示最新的数据"),
    ("half the visible time", "reducir a la mitad el tiempo visible", "可见时间减半"),
    ("twice the visible time", "duplicar el tiempo visible", "可见时间加倍"),
    ("zoom in as far as it goes", "acercar al máximo posible", "放大到极限"),
    ("show the whole capture", "mostrar la captura completa", "显示整个采集"),
    ("a third of the view earlier", "un tercio de la vista antes", "向前移动三分之一视图"),
    ("a third of the view later", "un tercio de la vista después", "向后移动三分之一视图"),
    ("a whole view earlier", "una vista completa antes", "向前移动一个视图"),
    ("a whole view later", "una vista completa después", "向后移动一个视图"),
    ("a page of rows up", "una página de filas arriba", "向上滚动一页行"),
    ("a page of rows down", "una página de filas abajo", "向下滚动一页行"),
    ("show the trigger in the middle", "mostrar el disparo en el centro", "在中间显示触发"),
    ("drag across a time range", "arrastrar sobre un rango de tiempo", "拖拽一个时间范围"),
    ("decode a protocol on a row", "decodificar un protocolo en una fila", "在一行上解码协议"),
    ("stop decoding until resumed", "detener la decodificación hasta reanudar", "停止解码，直到恢复"),
    ("the hovered analog row", "la fila analógica señalada", "悬停的模拟行"),
    ("fit the hovered analog row to its visible data", "ajustar la fila analógica señalada", "将悬停模拟行适应到可见数据"),
    ("drag a box", "arrastrar un recuadro", "拖拽一个框"),
    ("drag up or down", "arrastrar arriba o abajo", "上下拖拽"),
    ("fit every analog row to its visible data", "ajustar todas las filas analógicas", "将所有模拟行适应到可见数据"),
    ("another capture on the same device", "otra captura en el mismo dispositivo", "在同一设备上再采集一次"),
    ("close the active session", "cerrar la sesión activa", "关闭当前会话"),
    ("list the open sessions", "listar las sesiones abiertas", "列出打开的会话"),
    ("switch to the next session", "cambiar a la sesión siguiente", "切换到下一个会话"),
    ("switch to the previous session", "cambiar a la sesión anterior", "切换到上一个会话"),
    ("switch to a session by name", "cambiar a una sesión por nombre", "按名称切换会话"),
    ("rename the active session", "renombrar la sesión activa", "重命名当前会话"),
    ("keep only the active session", "conservar solo la sesión activa", "仅保留当前会话"),
    ("place a marker at the pointer", "colocar un marcador en el puntero", "在指针处放置标记"),
    ("two linked markers", "dos marcadores enlazados", "两个关联的标记"),
    ("the selected markers and ranges", "los marcadores y rangos seleccionados", "所选标记和范围"),
    ("in the open Timing Markers panel", "en el panel de marcadores abierto", "在打开的时序标记面板中"),
    ("measure between two times", "medir entre dos tiempos", "在两个时间之间测量"),
    ("wrap the pulse under the pointer in a range", "encerrar el pulso bajo el puntero", "将指针下方的脉冲圈为范围"),
    ("show or hide the Inspector", "mostrar u ocultar el inspector", "显示或隐藏检查器"),
    ("open the Inspector on the Device tab", "abrir el inspector en la pestaña Dispositivo", "在设备标签页打开检查器"),
    ("open the Inspector on the Analyzers tab", "abrir el inspector en la pestaña Analizadores", "在分析器标签页打开检查器"),
    ("open the Inspector on the Measurements tab", "abrir el inspector en la pestaña Mediciones", "在测量标签页打开检查器"),
    ("open the Inspector on the Markers tab", "abrir el inspector en la pestaña Marcadores", "在标记标签页打开检查器"),
    ("show or hide the Notes panel", "mostrar u ocultar el panel de notas", "显示或隐藏备注面板"),
    ("move the Inspector to a window of its own, or back", "mover el inspector a una ventana propia, o volver", "将检查器移到独立窗口，或移回"),
    ("every key binding", "todos los atajos de teclado", "所有按键绑定"),
    ("when a device isn't found", "cuando no se encuentra un dispositivo", "找不到设备时"),
    ("version, libraries and licenses", "versión, bibliotecas y licencias", "版本、库和许可证"),
    ("the welcome wizard: theme, device access, tips", "el asistente de bienvenida: tema, acceso, consejos", "欢迎向导：主题、设备访问、提示"),
    ("search the decoded data", "buscar en los datos decodificados", "搜索解码数据"),
    ("color waveforms by channel, or neutral", "colorear formas de onda por canal, o neutro", "按通道着色波形，或中性色"),
    ("show the live pin levels in the row labels", "mostrar los niveles en vivo en las etiquetas", "在行标签中显示实时引脚电平"),
    ("thin, normal or bold traces", "trazas finas, normales o gruesas", "细、正常或粗的波形"),
    ("light, dark or like the system", "claro, oscuro o como el sistema", "浅色、深色或跟随系统"),
    ("theme, UI scale, folders and more", "tema, escala de la interfaz, carpetas y más", "主题、界面缩放、文件夹等"),
    ("make text and controls bigger", "agrandar texto y controles", "放大文本和控件"),
    ("make text and controls smaller", "reducir texto y controles", "缩小文本和控件"),
    ("back to 100 %", "volver al 100 %", "回到 100 %"),
    ("run any command by name", "ejecutar cualquier comando por nombre", "按名称运行任意命令"),
    ("forget the palette's recent commands", "olvidar los comandos recientes", "清除面板的最近命令"),
    ("undo adding or deleting range measurements", "deshacer añadir o eliminar mediciones", "撤销添加或删除范围测量"),
    ("redo adding or deleting range measurements", "rehacer añadir o eliminar mediciones", "重做添加或删除范围测量"),
    ("keep only the visible range", "conservar solo el rango visible", "仅保留可见范围"),
    ("delete what lies left of the view", "eliminar lo que queda a la izquierda", "删除视图左侧的数据"),
    ("delete what lies right of the view", "eliminar lo que queda a la derecha", "删除视图右侧的数据"),
    ("back to the previous zoom and position", "volver al zoom y posición anteriores", "回到上一个缩放和位置"),
    ("forward again", "avanzar de nuevo", "再次前进"),

    // ---- Gesture / key hints ----
    ("Scroll over the labels", "Desplazarse sobre las etiquetas", "在标签上滚动"),
    ("CmdOrCtrl+Shift+Drag Horizontal", "CmdOrCtrl+Mayús+Arrastrar horizontal", "Cmd/Ctrl+Shift+水平拖拽"),
    ("CmdOrCtrl+Scroll", "CmdOrCtrl+Desplazar", "Cmd/Ctrl+滚动"),
    ("CmdOrCtrl+Drag", "CmdOrCtrl+Arrastrar", "Cmd/Ctrl+拖拽"),
    ("CmdOrCtrl+Shift+Drag Vertical", "CmdOrCtrl+Mayús+Arrastrar vertical", "Cmd/Ctrl+Shift+垂直拖拽"),
    ("CmdOrCtrl+1 … 9", "CmdOrCtrl+1 … 9", "Cmd/Ctrl+1…9"),
    ("Shift+Drag", "Mayús+Arrastrar", "Shift+拖拽"),
    ("Backspace", "Retroceso", "退格"),
    ("Delete", "Suprimir", "删除"),
    ("CmdOrCtrl+A", "CmdOrCtrl+A", "Cmd/Ctrl+A"),

    // ---- Enums (theme, trace width, trigger, bus, radix) ----
    ("Dark", "Oscuro", "深色"),
    ("Light", "Claro", "浅色"),
    ("System", "Sistema", "系统"),
    ("Thin", "Fina", "细"),
    ("Normal", "Normal", "正常"),
    ("Bold", "Gruesa", "粗"),
    ("Rising Edge", "Flanco de subida", "上升沿"),
    ("Falling Edge", "Flanco de bajada", "下降沿"),
    ("Positive Pulse", "Pulso positivo", "正脉冲"),
    ("Negative Pulse", "Pulso negativo", "负脉冲"),
    ("Hex", "Hex", "十六进制"),
    ("Decimal", "Decimal", "十进制"),
    ("Binary", "Binario", "二进制"),
    ("No Data", "Sin datos", "无数据"),
    ("Wire MSB → Data MSB", "Hilo MSB → Dato MSB", "线 MSB → 数据 MSB"),
    ("Wire LSB → Data LSB", "Hilo LSB → Dato LSB", "线 LSB → 数据 LSB"),
    ("Native", "Nativo", "原生"),
    ("Hexadecimal", "Hexadecimal", "十六进制"),
    ("Ascii", "Ascii", "ASCII"),

    // ---- Settings sections ----
    ("Appearance", "Apariencia", "外观"),
    ("Waveform", "Forma de onda", "波形"),
    ("Time Labels", "Etiquetas de tiempo", "时间标签"),
    ("Files & Folders", "Archivos y carpetas", "文件和文件夹"),
    ("Decoders", "Decodificadores", "解码器"),
    ("Data & History", "Datos e historial", "数据与历史"),
    ("Related", "Relacionado", "相关"),

    // ---- Start screen ----
    ("Devices", "Dispositivos", "设备"),
    ("Scanning for devices…", "Buscando dispositivos…", "正在扫描设备…"),
    ("Release Notes", "Notas de la versión", "发行说明"),
    ("Report Issue", "Informar de un problema", "报告问题"),
    ("User Guide", "Guía de usuario", "用户指南"),

    // ---- Common dialogs / buttons ----
    ("Save", "Guardar", "保存"),
    ("Cancel", "Cancelar", "取消"),
    ("Open", "Abrir", "打开"),
    ("Close", "Cerrar", "关闭"),
    ("Delete", "Eliminar", "删除"),
    ("Rename", "Renombrar", "重命名"),
    ("OK", "Aceptar", "确定"),
    ("Yes", "Sí", "是"),
    ("No", "No", "否"),
    ("Copy", "Copiar", "复制"),
    ("Export", "Exportar", "导出"),
    ("Import", "Importar", "导入"),
    ("Settings", "Ajustes", "设置"),
    ("Language", "Idioma", "语言"),

    // ---- State-dependent command labels (command_label) ----
    ("Start Recording", "Iniciar grabación", "开始录制"),
    ("Stop Recording", "Detener grabación", "停止录制"),
    ("Theme: {}", "Tema: {}", "主题：{}"),
    ("Trace Width: {}", "Grosor de traza: {}", "波形粗细：{}"),
    ("Pause Analyzers: {}", "Pausar analizadores: {}", "暂停分析器：{}"),
    ("Repeat Capture: {}", "Repetir captura: {}", "重复采集：{}"),
    ("Wire Status: {}", "Estado de los pines: {}", "连线状态：{}"),
    ("Toggle Traces Color: {}", "Alternar color de trazas: {}", "切换波形颜色：{}"),
    ("Go to Session 1…9", "Ir a la sesión 1…9", "转到会话 1…9"),
    ("on", "activado", "开"),
    ("off", "desactivado", "关"),
    ("neutral", "neutro", "中性"),
    ("by channel", "por canal", "按通道"),
    ("Color Waveforms by Channel", "Colorear formas de onda por canal", "按通道着色波形"),

    // ---- Menu bar ----
    ("File", "Archivo", "文件"),
    ("Edit", "Edición", "编辑"),
    ("View", "Ver", "视图"),
    ("Analyze", "Analizar", "分析"),
    ("Help", "Ayuda", "帮助"),
    ("Open Recent", "Abrir reciente", "打开最近"),
    ("Delete Data", "Eliminar datos", "删除数据"),
    ("Clear Recent", "Borrar recientes", "清除最近"),
    ("Quit", "Salir", "退出"),
    ("Settings…", "Ajustes…", "设置…"),
    ("Select All", "Seleccionar todo", "全选"),
    ("Off Screen", "Fuera de pantalla", "屏幕外"),
    ("Before the View", "Antes de la vista", "视图之前"),
    ("After the View", "Después de la vista", "视图之后"),
    ("Zoom In", "Acercar", "放大"),
    ("Zoom Out", "Alejar", "缩小"),
    ("Jump to Time…", "Ir a un tiempo…", "跳转到时间…"),
    ("Center on Trigger", "Centrar en el disparo", "居中于触发"),
    ("Show/Hide Inspector", "Mostrar/ocultar inspector", "显示/隐藏检查器"),
    ("Device", "Dispositivo", "设备"),
    ("Analyzers", "Analizadores", "分析器"),
    ("Add Marker Pair", "Añadir par de marcadores", "添加标记对"),
    ("Keyboard Shortcuts", "Atajos de teclado", "键盘快捷键"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_falls_back() {
        set(Language::Es);
        assert_eq!(tr("Save"), "Guardar");
        assert_eq!(tr("Capture"), "Captura");
        assert_eq!(tr("this string is not translated"), "this string is not translated");

        set(Language::ZhHans);
        assert_eq!(tr("Save"), "保存");
        assert_eq!(tr("Theme"), "主题");

        set(Language::En);
        assert_eq!(tr("Save"), "Save");
        assert_eq!(current(), Language::En);
    }
}
