#[cfg(target_arch = "wasm32")]
mod web_app {
    use std::cell::RefCell;

    use gloo_net::http::Request;
    use js_sys::Date;
    use plotters::prelude::*;
    use plotters::style::text_anchor::{HPos, Pos, VPos};
    use plotters_canvas::CanvasBackend;
    use serde::Deserialize;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{
        Document, Event, HtmlCanvasElement, HtmlElement, HtmlInputElement, HtmlSelectElement,
        KeyboardEvent, MessageEvent, MouseEvent, Storage, WebSocket, WheelEvent,
    };

    #[derive(Debug, Clone, Deserialize)]
    struct Candle {
        timestamp: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct RealtimeUpdateEvent {
        db: String,
        base: String,
        quote: String,
        candle: Candle,
    }

    thread_local! {
        static LAST_CANDLES: RefCell<Vec<Candle>> = const { RefCell::new(Vec::new()) };
        static LAST_RENDERED_CANDLES: RefCell<Vec<Candle>> = const { RefCell::new(Vec::new()) };
        static LIVE_WS: RefCell<Option<LiveWsConnection>> = const { RefCell::new(None) };
        static CLIENT_VIEW_RANGE: RefCell<Option<(i64, i64)>> = const { RefCell::new(None) };
        static PAN_LAST_X: RefCell<Option<i32>> = const { RefCell::new(None) };
        static DRAG_PAN_REMAINDER: RefCell<f64> = const { RefCell::new(0.0) };
        static WHEEL_PAN_REMAINDER: RefCell<f64> = const { RefCell::new(0.0) };
        static STRETCH_TOOL_ENABLED: RefCell<bool> = const { RefCell::new(false) };
        static AUTO_MODE_ENABLED: RefCell<bool> = const { RefCell::new(false) };
        static CHART_DRAG: RefCell<Option<ChartDragState>> = const { RefCell::new(None) };
        static MEASURE_STATE: RefCell<MeasureState> = const { RefCell::new(MeasureState::new()) };
        static Y_STRETCH_FACTOR: RefCell<f64> = const { RefCell::new(1.0) };
        static Y_PAN_LINEAR_OFFSET: RefCell<f64> = const { RefCell::new(0.0) };
        static Y_PAN_LOG_OFFSET: RefCell<f64> = const { RefCell::new(0.0) };
        static Y_STRETCH_DRAG: RefCell<Option<YStretchDrag>> = const { RefCell::new(None) };
        static FIB_STATE: RefCell<FibState> = const { RefCell::new(FibState::new()) };
        // Completed fib retracements (each a pair of anchor points).
        static FIB_LINES: RefCell<Vec<((i64, f64), (i64, f64))>> = const { RefCell::new(Vec::new()) };
        static FIB_LEVEL_DRAG: RefCell<Option<FibLevelDrag>> = const { RefCell::new(None) };
        static FIB_PREVIEW_POINT: RefCell<Option<(i64, f64)>> = const { RefCell::new(None) };
        static MEASURE_DRAG_TS: RefCell<Option<i64>> = const { RefCell::new(None) };
        static MEASURE_DRAG_PRICE: RefCell<Option<f64>> = const { RefCell::new(None) };
        static FIB_POPUP_DRAG: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
        static LINE_TOOL_ENABLED: RefCell<bool> = const { RefCell::new(false) };
        static LINE_DRAFT_ANCHOR: RefCell<Option<(i64, f64)>> = const { RefCell::new(None) };
        static LINE_PREVIEW_POINT: RefCell<Option<(i64, f64)>> = const { RefCell::new(None) };
        static TREND_LINES: RefCell<Vec<((i64, f64), (i64, f64))>> = const { RefCell::new(Vec::new()) };
        static MA_SETTINGS_DRAG: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
        static CONNECTION_SETTINGS_DRAG: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
        static CHART_VIEW: RefCell<Option<ChartView>> = const { RefCell::new(None) };
        // The last price currently shown in the floating price tag, so we can
        // flash it green/red only when the value actually changes.
        static LAST_PRICE_TAG_VALUE: RefCell<Option<f64>> = const { RefCell::new(None) };
        static RANGE_HISTORY: RefCell<Vec<(i64, i64)>> = const { RefCell::new(Vec::new()) };
        // Which chart-source the in-memory FIB_STATE / TREND_LINES currently belong to,
        // so drawings can be swapped out and persisted per pair on chart switches.
        static CURRENT_PAIR_KEY: RefCell<String> = const { RefCell::new(String::new()) };
        // The figure currently under the hover trash icon, the pending hide timer
        // handle, and the reusable timeout callback that hides the icon.
        static FIGURE_TRASH_TARGET: RefCell<Option<FigureTarget>> = const { RefCell::new(None) };
        static FIGURE_TRASH_HIDE_TIMER: RefCell<Option<i32>> = const { RefCell::new(None) };
        static FIGURE_TRASH_HIDE_CLOSURE: RefCell<Option<Closure<dyn FnMut()>>> =
            const { RefCell::new(None) };
        static CHART_FLIPPED: RefCell<bool> = const { RefCell::new(false) };
    }

    const STORAGE_KEY_API_BASE: &str = "price_api.api_base";
    const STORAGE_KEY_DB: &str = "price_api.db";
    const STORAGE_KEY_CHART_SOURCE: &str = "price_api.chart_source";
    const STORAGE_KEY_PERIOD: &str = "price_api.period";
    const STORAGE_KEY_TS_START: &str = "price_api.ts_start_human";
    const STORAGE_KEY_TS_END: &str = "price_api.ts_end_human";
    const STORAGE_KEY_LOG_SCALE: &str = "price_api.log_scale";
    const STORAGE_KEY_SETTINGS_VISIBLE: &str = "price_api.settings_visible";
    const STORAGE_KEY_SETTINGS_SIDE: &str = "price_api.settings_side";
    const STORAGE_KEY_CONNECTION_SETTINGS_VISIBLE: &str = "price_api.connection_settings_visible";
    const STORAGE_KEY_CONNECTION_SETTINGS_SIDE: &str = "price_api.connection_settings_side";
    const STORAGE_KEY_VIEW_START: &str = "price_api.view_start";
    const STORAGE_KEY_VIEW_END: &str = "price_api.view_end";
    const STORAGE_KEY_VIEW_PERIOD: &str = "price_api.view_period";
    const STORAGE_KEY_FIB_PREFIX: &str = "price_api.fib.";
    const STORAGE_KEY_LINES_PREFIX: &str = "price_api.lines.";
    const STORAGE_KEY_CHART_FLIPPED: &str = "price_api.chart_flipped";
    const MA_COUNT: usize = 15;

    #[derive(Clone, Copy)]
    struct MovingAverageConfig {
        idx: usize,
        enabled: bool,
        period: usize,
        color: RGBColor,
    }

    #[derive(Clone, Copy)]
    struct ChartView {
        x_start: i64,
        x_end: i64,
        y_low: f64,
        y_high: f64,
        use_log_scale: bool,
        flipped: bool,
    }

    #[derive(Clone, Copy)]
    struct YStretchDrag {
        start_y: i32,
        start_factor: f64,
    }

    // Which anchor (100% or 0% line) of which completed fib is being dragged.
    #[derive(Clone, Copy)]
    enum FibLevelDrag {
        AnchorA(usize),
        AnchorB(usize),
    }

    impl FibLevelDrag {
        fn index(self) -> usize {
            match self {
                FibLevelDrag::AnchorA(idx) | FibLevelDrag::AnchorB(idx) => idx,
            }
        }
    }

    // A drawn figure the on-chart trash icon can delete when hovered.
    #[derive(Clone, Copy, PartialEq)]
    enum FigureTarget {
        Fib(usize),
        Measure,
        TrendLine(usize),
    }

    struct LiveWsConnection {
        ws: WebSocket,
        _onopen: Closure<dyn FnMut(Event)>,
        _onmessage: Closure<dyn FnMut(MessageEvent)>,
        _onerror: Closure<dyn FnMut(Event)>,
        _onclose: Closure<dyn FnMut(Event)>,
    }

    #[derive(Clone, Copy)]
    struct ChartDragState {
        start_x: i32,
        start_y: i32,
        ts_start: i64,
        ts_end: i64,
        y_offset_start: f64,
        y_span: f64,
        use_log_scale: bool,
    }

    #[derive(Clone, Copy)]
    struct MeasureState {
        enabled: bool,
        anchor_a: Option<(i64, f64)>,
        anchor_b: Option<(i64, f64)>,
    }

    impl MeasureState {
        const fn new() -> Self {
            Self {
                enabled: false,
                anchor_a: None,
                anchor_b: None,
            }
        }
    }

    // The fib tool state: whether it is active and the first clicked point of an
    // in-progress fib (the completed fibs live in FIB_LINES).
    #[derive(Clone, Copy)]
    struct FibState {
        enabled: bool,
        draft: Option<(i64, f64)>,
    }

    impl FibState {
        const fn new() -> Self {
            Self {
                enabled: false,
                draft: None,
            }
        }
    }

    #[derive(Clone)]
    struct FibOverlay {
        x_start: i64,
        x_end: i64,
        levels: Vec<(f64, f64)>,
    }

    // Fib retracement levels shared by every fib overlay.
    const FIB_RATIOS: [f64; 11] = [
        0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0, 1.681, 2.618, 3.618, 4.236,
    ];

    // Build a fib overlay from its two anchor points (a = 100% line, b = 0% line).
    fn fib_overlay_from_anchors(a: (i64, f64), b: (i64, f64)) -> FibOverlay {
        let ((ts_a, price_a), (ts_b, price_b)) = (a, b);
        let delta = price_b - price_a;
        FibOverlay {
            x_start: ts_a.min(ts_b),
            x_end: ts_a.max(ts_b),
            levels: FIB_RATIOS
                .into_iter()
                .map(|r| (r, price_b - delta * r))
                .collect(),
        }
    }

    fn document() -> Result<Document, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window is not available"))?
            .document()
            .ok_or_else(|| JsValue::from_str("document is not available"))
    }

    fn input_value(id: &str) -> Result<String, JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing input element"))?
            .dyn_into::<HtmlInputElement>()?;
        Ok(input.value())
    }

    fn select_value(id: &str) -> Result<String, JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing select element"))?
            .dyn_into::<HtmlSelectElement>()?;
        Ok(input.value())
    }

    fn set_input_value(id: &str, value: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing input element"))?
            .dyn_into::<HtmlInputElement>()?;
        input.set_value(value);
        Ok(())
    }

    fn set_select_value(id: &str, value: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let select = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing select element"))?
            .dyn_into::<HtmlSelectElement>()?;
        select.set_value(value);
        Ok(())
    }

    fn set_checkbox_checked(id: &str, checked: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing checkbox element"))?
            .dyn_into::<HtmlInputElement>()?;
        input.set_checked(checked);
        Ok(())
    }

    fn checkbox_checked(id: &str) -> Result<bool, JsValue> {
        let doc = document()?;
        let input = doc
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str("missing checkbox element"))?
            .dyn_into::<HtmlInputElement>()?;
        Ok(input.checked())
    }

    fn sync_log_scale_button() -> Result<(), JsValue> {
        let enabled = checkbox_checked("log-scale")?;
        let doc = document()?;
        let button = doc
            .get_element_by_id("log-scale-toggle")
            .ok_or_else(|| JsValue::from_str("missing log scale toggle button"))?;

        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_attribute("aria-pressed", "true")?;
            button.set_attribute("aria-label", "Log On")?;
            button.set_attribute("title", "Log On")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_attribute("aria-pressed", "false")?;
            button.set_attribute("aria-label", "Log Off")?;
            button.set_attribute("title", "Log Off")?;
        }

        Ok(())
    }

    fn sync_flip_button() -> Result<(), JsValue> {
        let flipped = CHART_FLIPPED.with(|state| *state.borrow());
        let doc = document()?;
        let button = doc
            .get_element_by_id("flip-chart")
            .ok_or_else(|| JsValue::from_str("missing flip chart button"))?;
        if flipped {
            button.set_class_name("toggle-btn active");
            button.set_attribute("aria-pressed", "true")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_attribute("aria-pressed", "false")?;
        }
        Ok(())
    }

    fn format_price_short(price: f64) -> String {
        if price >= 10_000.0 {
            format!("{:.0}", price)
        } else if price >= 100.0 {
            format!("{:.2}", price)
        } else {
            format!("{:.4}", price)
        }
    }

    fn sync_drawings_panel() {
        let Ok(doc) = document() else { return };
        let fibs = FIB_LINES.with(|state| state.borrow().clone());
        let lines = TREND_LINES.with(|state| state.borrow().clone());

        if let Some(fib_list) = doc.get_element_by_id("fib-list") {
            if fibs.is_empty() {
                fib_list.set_inner_html("<span class=\"drawings-empty\">None</span>");
            } else {
                let html: String = fibs
                    .iter()
                    .enumerate()
                    .map(|(i, ((_, a_price), (_, b_price)))| {
                        format!(
                            "<div class=\"drawing-row\"><span class=\"drawing-label\">Fib {}: {} → {}</span><button class=\"drawing-delete\" data-fib-idx=\"{}\" type=\"button\">✕</button></div>",
                            i + 1,
                            format_price_short(*a_price),
                            format_price_short(*b_price),
                            i,
                        )
                    })
                    .collect();
                fib_list.set_inner_html(&html);
            }
        }

        if let Some(lines_list) = doc.get_element_by_id("lines-list") {
            if lines.is_empty() {
                lines_list.set_inner_html("<span class=\"drawings-empty\">None</span>");
            } else {
                let html: String = lines
                    .iter()
                    .enumerate()
                    .map(|(i, ((_, a_price), (_, b_price)))| {
                        format!(
                            "<div class=\"drawing-row\"><span class=\"drawing-label\">Line {}: {} → {}</span><button class=\"drawing-delete\" data-line-idx=\"{}\" type=\"button\">✕</button></div>",
                            i + 1,
                            format_price_short(*a_price),
                            format_price_short(*b_price),
                            i,
                        )
                    })
                    .collect();
                lines_list.set_inner_html(&html);
            }
        }
    }

    fn sync_fib_button() -> Result<(), JsValue> {
        let doc = document()?;
        let button = doc
            .get_element_by_id("fib-toggle")
            .ok_or_else(|| JsValue::from_str("missing fib toggle button"))?;

        let enabled = FIB_STATE.with(|state| state.borrow().enabled);
        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_text_content(Some("Fib On"));
            button.set_attribute("aria-pressed", "true")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_text_content(Some("Fib Off"));
            button.set_attribute("aria-pressed", "false")?;
        }

        Ok(())
    }

    fn sync_stretch_button() -> Result<(), JsValue> {
        let doc = document()?;
        let button = doc
            .get_element_by_id("stretch-toggle")
            .ok_or_else(|| JsValue::from_str("missing stretch toggle button"))?;

        let enabled = STRETCH_TOOL_ENABLED.with(|state| *state.borrow());
        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_attribute("aria-pressed", "true")?;
            button.set_attribute("aria-label", "Stretch On")?;
            button.set_attribute("title", "Stretch On")?;
            set_chart_cursor("ns-resize");
        } else {
            button.set_class_name("toggle-btn");
            button.set_attribute("aria-pressed", "false")?;
            button.set_attribute("aria-label", "Stretch Off")?;
            button.set_attribute("title", "Stretch Off")?;
            set_chart_cursor("grab");
        }

        Ok(())
    }

    fn sync_auto_mode_button() -> Result<(), JsValue> {
        let doc = document()?;
        let button = doc
            .get_element_by_id("auto-mode-toggle")
            .ok_or_else(|| JsValue::from_str("missing auto mode toggle button"))?;

        let enabled = AUTO_MODE_ENABLED.with(|state| *state.borrow());
        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_attribute("aria-pressed", "true")?;
            button.set_attribute("aria-label", "Auto On")?;
            button.set_attribute("title", "Auto On")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_attribute("aria-pressed", "false")?;
            button.set_attribute("aria-label", "Auto Off")?;
            button.set_attribute("title", "Auto Off")?;
        }

        Ok(())
    }

    fn sync_measure_button() -> Result<(), JsValue> {
        let doc = document()?;
        let button = doc
            .get_element_by_id("measure-toggle")
            .ok_or_else(|| JsValue::from_str("missing measure toggle button"))?;

        let enabled = MEASURE_STATE.with(|state| state.borrow().enabled);
        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_attribute("aria-pressed", "true")?;
            button.set_attribute("aria-label", "Price Percent On")?;
            button.set_attribute("title", "Price Percent On")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_attribute("aria-pressed", "false")?;
            button.set_attribute("aria-label", "Price Percent Off")?;
            button.set_attribute("title", "Price Percent Off")?;
        }

        Ok(())
    }

    // Turn off the interactive chart tools (Stretch, Price %, Fib, Line) without
    // discarding their finished drawings. Used by the Escape key to deactivate
    // whatever tool is on while keeping the fib, lines and price range on screen.
    fn cancel_active_tools() -> Result<(), JsValue> {
        STRETCH_TOOL_ENABLED.with(|state| {
            *state.borrow_mut() = false;
        });
        MEASURE_STATE.with(|state| {
            state.borrow_mut().enabled = false;
        });
        MEASURE_DRAG_TS.with(|state| {
            *state.borrow_mut() = None;
        });
        MEASURE_DRAG_PRICE.with(|state| {
            *state.borrow_mut() = None;
        });
        FIB_STATE.with(|state| {
            state.borrow_mut().enabled = false;
        });
        let _ = set_fib_preview_point(None);
        LINE_TOOL_ENABLED.with(|state| {
            *state.borrow_mut() = false;
        });
        LINE_DRAFT_ANCHOR.with(|state| {
            *state.borrow_mut() = None;
        });
        LINE_PREVIEW_POINT.with(|state| {
            *state.borrow_mut() = None;
        });
        sync_stretch_button()?;
        sync_measure_button()?;
        sync_fib_button()?;
        sync_line_button()?;
        Ok(())
    }

    // Disable the other interactive tools (Stretch, Price %, Fib) when the line
    // tool is turned on, mirroring how the Fib toggle clears the rest.
    fn disable_tools_for_line() -> Result<(), JsValue> {
        STRETCH_TOOL_ENABLED.with(|state| {
            *state.borrow_mut() = false;
        });
        MEASURE_STATE.with(|state| {
            let mut cfg = state.borrow_mut();
            cfg.enabled = false;
            cfg.anchor_a = None;
            cfg.anchor_b = None;
        });
        MEASURE_DRAG_TS.with(|state| {
            *state.borrow_mut() = None;
        });
        MEASURE_DRAG_PRICE.with(|state| {
            *state.borrow_mut() = None;
        });
        FIB_STATE.with(|state| {
            state.borrow_mut().enabled = false;
        });
        let _ = set_fib_preview_point(None);
        sync_stretch_button()?;
        sync_measure_button()?;
        sync_fib_button()?;
        Ok(())
    }

    // Turn the line tool off and drop any in-progress draft, used when another
    // tool (Drag, Price %, Fib) is switched on so only one tool is active.
    fn disable_line_tool() -> Result<(), JsValue> {
        LINE_TOOL_ENABLED.with(|state| {
            *state.borrow_mut() = false;
        });
        LINE_DRAFT_ANCHOR.with(|state| {
            *state.borrow_mut() = None;
        });
        LINE_PREVIEW_POINT.with(|state| {
            *state.borrow_mut() = None;
        });
        sync_line_button()
    }

    fn sync_line_button() -> Result<(), JsValue> {
        let doc = document()?;
        let button = doc
            .get_element_by_id("line-toggle")
            .ok_or_else(|| JsValue::from_str("missing line toggle button"))?;

        let enabled = LINE_TOOL_ENABLED.with(|state| *state.borrow());
        if enabled {
            button.set_class_name("toggle-btn active");
            button.set_attribute("aria-pressed", "true")?;
            button.set_attribute("aria-label", "Line On")?;
            button.set_attribute("title", "Line On")?;
        } else {
            button.set_class_name("toggle-btn");
            button.set_attribute("aria-pressed", "false")?;
            button.set_attribute("aria-label", "Line Off")?;
            button.set_attribute("title", "Line Off")?;
        }

        Ok(())
    }

    fn clear_trend_lines() {
        TREND_LINES.with(|state| {
            state.borrow_mut().clear();
        });
        LINE_DRAFT_ANCHOR.with(|state| {
            *state.borrow_mut() = None;
        });
        LINE_PREVIEW_POINT.with(|state| {
            *state.borrow_mut() = None;
        });
    }

    // chart-source select value + flip state, used as the key for per-pair drawing storage.
    // Flipped and non-flipped charts keep independent drawing sets.
    fn current_pair_key() -> String {
        let pair = select_value("chart-source").unwrap_or_default();
        let flipped = CHART_FLIPPED.with(|s| *s.borrow());
        if flipped {
            format!("{pair}_flipped")
        } else {
            pair
        }
    }

    fn serialize_segments(segments: &[((i64, f64), (i64, f64))]) -> String {
        segments
            .iter()
            .map(|((a_ts, a_price), (b_ts, b_price))| {
                format!("{a_ts},{a_price},{b_ts},{b_price}")
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    fn parse_segments(value: &str) -> Vec<((i64, f64), (i64, f64))> {
        value
            .split(';')
            .filter(|s| !s.is_empty())
            .filter_map(|segment| {
                let parts: Vec<&str> = segment.split(',').collect();
                parse_drawing_point(&parts)
            })
            .collect()
    }

    // Serialize the finished fibs and trend lines for `pair` into localStorage so
    // each chart source keeps its own drawings across switches and reloads.
    fn persist_pair_drawings(pair: &str) {
        if pair.is_empty() {
            return;
        }
        let Ok(storage) = storage() else {
            return;
        };
        let fib_value = FIB_LINES.with(|state| serialize_segments(&state.borrow()));
        let _ = storage.set_item(&format!("{STORAGE_KEY_FIB_PREFIX}{pair}"), &fib_value);

        let lines_value = TREND_LINES.with(|state| serialize_segments(&state.borrow()));
        let _ = storage.set_item(&format!("{STORAGE_KEY_LINES_PREFIX}{pair}"), &lines_value);
    }

    // Persist whatever pair the in-memory drawings currently belong to.
    fn persist_current_pair_drawings() {
        let pair = CURRENT_PAIR_KEY.with(|cur| cur.borrow().clone());
        persist_pair_drawings(&pair);
    }

    fn parse_drawing_point(parts: &[&str]) -> Option<((i64, f64), (i64, f64))> {
        if parts.len() != 4 {
            return None;
        }
        Some((
            (parts[0].parse().ok()?, parts[1].parse().ok()?),
            (parts[2].parse().ok()?, parts[3].parse().ok()?),
        ))
    }

    // Replace the in-memory fibs + trend lines with the drawings saved for `pair`,
    // discarding any in-progress draft from the previous pair.
    fn load_pair_drawings(pair: &str) {
        clear_trend_lines();
        clear_measure();
        clear_fib_levels();

        let Ok(storage) = storage() else {
            return;
        };

        if let Ok(Some(value)) = storage.get_item(&format!("{STORAGE_KEY_FIB_PREFIX}{pair}")) {
            let fibs = parse_segments(&value);
            FIB_LINES.with(|state| {
                *state.borrow_mut() = fibs;
            });
        }

        if let Ok(Some(value)) = storage.get_item(&format!("{STORAGE_KEY_LINES_PREFIX}{pair}")) {
            let lines = parse_segments(&value);
            TREND_LINES.with(|state| {
                *state.borrow_mut() = lines;
            });
        }
        sync_drawings_panel();
    }

    // Save the current pair's drawings then load the drawings belonging to
    // `next_pair`, updating the tracked current pair.
    fn switch_pair_drawings(next_pair: &str) {
        let previous = CURRENT_PAIR_KEY.with(|cur| cur.borrow().clone());
        if previous == next_pair {
            return;
        }
        persist_pair_drawings(&previous);
        load_pair_drawings(next_pair);
        CURRENT_PAIR_KEY.with(|cur| {
            *cur.borrow_mut() = next_pair.to_string();
        });
    }

    // Finished trend lines plus, while the tool is mid-draw, a live segment from
    // the first clicked anchor to the current cursor preview point.
    fn active_trend_lines() -> Vec<((i64, f64), (i64, f64))> {
        let mut lines = TREND_LINES.with(|state| state.borrow().clone());
        if let Some(anchor) = LINE_DRAFT_ANCHOR.with(|state| *state.borrow()) {
            if let Some(preview) = LINE_PREVIEW_POINT.with(|state| *state.borrow()) {
                lines.push((anchor, preview));
            }
        }
        lines
    }

    fn set_load_button_loading(loading: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let button = doc
            .get_element_by_id("load")
            .ok_or_else(|| JsValue::from_str("missing load button"))?
            .dyn_into::<HtmlElement>()?;

        if loading {
            button.set_class_name("loading");
            button.set_attribute("disabled", "true")?;
            button.set_attribute("aria-busy", "true")?;
        } else {
            button.set_class_name("");
            button.remove_attribute("disabled")?;
            button.set_attribute("aria-busy", "false")?;
        }

        Ok(())
    }

    // The live preview fib while the tool is mid-draw (first point placed, cursor
    // tracking the second), if any.
    fn fib_preview_anchors() -> Option<((i64, f64), (i64, f64))> {
        let cfg = FIB_STATE.with(|state| *state.borrow());
        if !cfg.enabled {
            return None;
        }
        let draft = cfg.draft?;
        let preview = FIB_PREVIEW_POINT.with(|preview| *preview.borrow())?;
        Some((draft, preview))
    }

    // All completed fibs plus, while drawing, the live preview fib. Used for
    // rendering only (hit-testing iterates FIB_LINES directly for indices).
    fn active_fib_overlays() -> Vec<FibOverlay> {
        let mut overlays: Vec<FibOverlay> = FIB_LINES.with(|state| {
            state
                .borrow()
                .iter()
                .map(|(a, b)| fib_overlay_from_anchors(*a, *b))
                .collect()
        });
        if let Some((a, b)) = fib_preview_anchors() {
            overlays.push(fib_overlay_from_anchors(a, b));
        }
        overlays
    }

    fn active_measure_range() -> Option<(i64, i64)> {
        MEASURE_STATE.with(|state| {
            let cfg = *state.borrow();
            let (start, _) = cfg.anchor_a?;
            let (end, _) = match (cfg.anchor_b, cfg.enabled) {
                (Some(v), _) => v,
                (None, true) => (
                    MEASURE_DRAG_TS.with(|drag| *drag.borrow())?,
                    MEASURE_DRAG_PRICE.with(|drag| *drag.borrow())?,
                ),
                (None, false) => return None,
            };
            Some((start.min(end), start.max(end)))
        })
    }

    fn active_measure_price_range() -> Option<(f64, f64)> {
        MEASURE_STATE.with(|state| {
            let cfg = *state.borrow();
            let (_, start) = cfg.anchor_a?;
            let (_, end) = match (cfg.anchor_b, cfg.enabled) {
                (Some(v), _) => v,
                (None, true) => (
                    MEASURE_DRAG_TS.with(|drag| *drag.borrow())?,
                    MEASURE_DRAG_PRICE.with(|drag| *drag.borrow())?,
                ),
                (None, false) => return None,
            };
            Some((start, end))
        })
    }

    fn set_fib_preview_point(next: Option<(i64, f64)>) -> bool {
        FIB_PREVIEW_POINT.with(|state| {
            let mut cur = state.borrow_mut();
            let changed = match (*cur, next) {
                (None, None) => false,
                (Some((cur_ts, cur_price)), Some((next_ts, next_price))) => {
                    cur_ts != next_ts || (cur_price - next_price).abs() > 0.01
                }
                _ => true,
            };
            if changed {
                *cur = next;
            }
            changed
        })
    }

    fn fib_level_drag_label(level: FibLevelDrag) -> &'static str {
        match level {
            FibLevelDrag::AnchorA(_) => "Fib 1.0 (100%)",
            FibLevelDrag::AnchorB(_) => "Fib 0.0 (0%)",
        }
    }

    fn finished_fib_level_price(level: FibLevelDrag) -> Option<f64> {
        FIB_LINES.with(|state| {
            let lines = state.borrow();
            let (a, b) = lines.get(level.index())?;
            match level {
                FibLevelDrag::AnchorA(_) => Some(a.1),
                FibLevelDrag::AnchorB(_) => Some(b.1),
            }
        })
    }

    fn set_fib_level_price(level: FibLevelDrag, next_price: f64) -> bool {
        FIB_LINES.with(|state| {
            let mut lines = state.borrow_mut();
            let Some((a, b)) = lines.get_mut(level.index()) else {
                return false;
            };
            let target = match level {
                FibLevelDrag::AnchorA(_) => a,
                FibLevelDrag::AnchorB(_) => b,
            };
            if (target.1 - next_price).abs() < 0.01 {
                return false;
            }
            target.1 = next_price;
            true
        })
    }

    fn canvas_y_from_price(price: f64, plot_top: f64, plot_bottom: f64) -> Option<f64> {
        if plot_bottom <= plot_top {
            return None;
        }

        CHART_VIEW.with(|view| {
            let cfg = (*view.borrow())?;
            let ratio = if cfg.use_log_scale {
                if cfg.y_low <= 0.0 || cfg.y_high <= 0.0 || price <= 0.0 {
                    return None;
                }
                let low_ln = cfg.y_low.ln();
                let high_ln = cfg.y_high.ln();
                if high_ln <= low_ln {
                    return None;
                }
                ((price.ln() - low_ln) / (high_ln - low_ln)).clamp(0.0, 1.0)
            } else {
                let span = cfg.y_high - cfg.y_low;
                if span.abs() < f64::EPSILON {
                    return None;
                }
                ((price - cfg.y_low) / span).clamp(0.0, 1.0)
            };
            let effective_ratio = if cfg.flipped { 1.0 - ratio } else { ratio };
            Some(plot_bottom - effective_ratio * (plot_bottom - plot_top))
        })
    }

    // Forward map of a timestamp to its canvas x (CSS px). Unlike
    // `timestamp_from_canvas_x` this does not clamp, so off-screen figure
    // endpoints keep their true geometry for hit-testing.
    fn canvas_x_from_timestamp(ts: i64, plot_left: f64, plot_right: f64) -> Option<f64> {
        if plot_right <= plot_left {
            return None;
        }
        CHART_VIEW.with(|view| {
            let cfg = (*view.borrow())?;
            let span = (cfg.x_end - cfg.x_start).max(60) as f64;
            let ratio = (ts - cfg.x_start) as f64 / span;
            Some(plot_left + ratio * (plot_right - plot_left))
        })
    }

    fn point_segment_distance(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
        let dx = bx - ax;
        let dy = by - ay;
        let len_sq = dx * dx + dy * dy;
        if len_sq <= f64::EPSILON {
            return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
        }
        let t = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
        let proj_x = ax + t * dx;
        let proj_y = ay + t * dy;
        ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
    }

    // Find the draggable fib anchor line (0% or 100% of some completed fib) under
    // the cursor, restricted to that fib's horizontal span.
    fn fib_level_hit_test(
        cursor_x: f64,
        offset_y: f64,
        plot_left: f64,
        plot_right: f64,
        plot_top: f64,
        plot_bottom: f64,
    ) -> Option<FibLevelDrag> {
        let (view_x_start, view_x_end) = CHART_VIEW.with(|view| {
            view.borrow()
                .map(|cfg| (cfg.x_start, cfg.x_end))
                .unwrap_or((0, 0))
        });
        let fibs = FIB_LINES.with(|state| state.borrow().clone());
        let mut best: Option<(FibLevelDrag, f64)> = None;
        for (idx, (a, b)) in fibs.iter().enumerate() {
            let (Some(xs), Some(xe)) = (
                canvas_x_from_timestamp(a.0.min(b.0).clamp(view_x_start, view_x_end), plot_left, plot_right),
                canvas_x_from_timestamp(a.0.max(b.0).clamp(view_x_start, view_x_end), plot_left, plot_right),
            ) else {
                continue;
            };
            if cursor_x < xs - 2.0 || cursor_x > xe + 2.0 {
                continue;
            }
            for level in [FibLevelDrag::AnchorA(idx), FibLevelDrag::AnchorB(idx)] {
                let price = if matches!(level, FibLevelDrag::AnchorA(_)) {
                    a.1
                } else {
                    b.1
                };
                let Some(line_y) = canvas_y_from_price(price, plot_top, plot_bottom) else {
                    continue;
                };
                let distance = (offset_y - line_y).abs();
                if distance <= 8.0 && best.as_ref().map(|(_, d)| distance < *d).unwrap_or(true) {
                    best = Some((level, distance));
                }
            }
        }
        best.map(|(level, _)| level)
    }

    fn redraw_visible_chart_only() -> Result<(), JsValue> {
        if AUTO_MODE_ENABLED.with(|s| *s.borrow()) {
            Y_STRETCH_FACTOR.with(|s| *s.borrow_mut() = 1.0);
            Y_PAN_LINEAR_OFFSET.with(|s| *s.borrow_mut() = 0.0);
            Y_PAN_LOG_OFFSET.with(|s| *s.borrow_mut() = 0.0);
        }
        let candles = LAST_RENDERED_CANDLES.with(|state| state.borrow().clone());
        if candles.is_empty() {
            return Ok(());
        }
        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        draw(&candles, log_scale, &ma_configs)
    }

    fn fib_ratio_label(ratio: f64) -> String {
        let text = format!("{ratio:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }

    fn format_duration_human(seconds: i64) -> String {
        let mut remaining = seconds.abs().max(1);
        let units = [
            ("days", 86_400_i64),
            ("hours", 3_600_i64),
            ("min", 60_i64),
            ("sec", 1_i64),
        ];
        let mut parts = Vec::new();
        for (label, size) in units {
            if remaining >= size || (size == 1 && parts.is_empty()) {
                let value = remaining / size;
                remaining %= size;
                if value > 0 {
                    parts.push(format!("{value}{label}"));
                }
            }
            if parts.len() == 2 {
                break;
            }
        }
        parts.join(" ")
    }

    // Per-fib data ready to render: x span clamped to the view, a label anchor,
    // and the levels currently within the visible price band.
    struct FibRender {
        x_start: i64,
        x_end: i64,
        label_x: i64,
        levels: Vec<(f64, f64)>,
    }

    fn fib_renders(
        x_start: i64,
        x_end: i64,
        y_low: f64,
        y_high: f64,
        log_scale: bool,
    ) -> Vec<FibRender> {
        active_fib_overlays()
            .into_iter()
            .map(|overlay| {
                let start = overlay.x_start.clamp(x_start, x_end);
                let end = overlay.x_end.clamp(x_start, x_end);
                let label_x = (start + ((end - start).max(60) / 40).max(1)).min(x_end);
                let levels = overlay
                    .levels
                    .into_iter()
                    .filter(|(_, level_price)| {
                        level_price.is_finite()
                            && (!log_scale || *level_price > 0.0)
                            && *level_price >= y_low
                            && *level_price <= y_high
                    })
                    .collect();
                FibRender {
                    x_start: start.min(end),
                    x_end: start.max(end),
                    label_x,
                    levels,
                }
            })
            .collect()
    }

    fn set_status(text: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("status") {
                node.set_text_content(Some(text));
            }
        }
    }

    fn set_hover_info(text: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("hover-info") {
                node.set_text_content(Some(text));
            }
        }
    }

    fn set_fib_popup_info(text: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("fib-popup-info") {
                node.set_text_content(Some(text));
            }
        }
    }

    // Remove all fibs and any in-progress draft.
    fn clear_fib_levels() {
        FIB_LINES.with(|state| {
            state.borrow_mut().clear();
        });
        FIB_STATE.with(|state| {
            state.borrow_mut().draft = None;
        });
        let _ = set_fib_preview_point(None);
    }

    fn clear_measure() {
        MEASURE_STATE.with(|state| {
            let mut cfg = state.borrow_mut();
            cfg.anchor_a = None;
            cfg.anchor_b = None;
        });
        MEASURE_DRAG_TS.with(|state| {
            *state.borrow_mut() = None;
        });
        MEASURE_DRAG_PRICE.with(|state| {
            *state.borrow_mut() = None;
        });
    }

    // Find the drawn figure (trend line, fib, or price range) nearest the cursor.
    // Returns the matched figure plus a point on it (canvas CSS px) where the
    // trash icon should sit.
    fn figure_hit_test(
        cursor_x: f64,
        cursor_y: f64,
        plot_left: f64,
        plot_right: f64,
        plot_top: f64,
        plot_bottom: f64,
    ) -> Option<(FigureTarget, f64, f64)> {
        const TOL: f64 = 7.0;
        // (target, distance, anchor_x, anchor_y)
        let mut candidates: Vec<(FigureTarget, f64, f64, f64)> = Vec::new();

        let (view_x_start, view_x_end, y_low, y_high) = CHART_VIEW.with(|view| {
            view.borrow()
                .map(|cfg| (cfg.x_start, cfg.x_end, cfg.y_low, cfg.y_high))
                .unwrap_or((0, 0, 0.0, 0.0))
        });
        let clamp_x = |x: f64| x.clamp(plot_left, plot_right);
        let clamp_y = |y: f64| y.clamp(plot_top, plot_bottom);

        let trend_lines = TREND_LINES.with(|state| state.borrow().clone());
        for (idx, ((a_ts, a_price), (b_ts, b_price))) in trend_lines.iter().enumerate() {
            let (Some(ax), Some(ay), Some(bx), Some(by)) = (
                canvas_x_from_timestamp(*a_ts, plot_left, plot_right),
                canvas_y_from_price(*a_price, plot_top, plot_bottom),
                canvas_x_from_timestamp(*b_ts, plot_left, plot_right),
                canvas_y_from_price(*b_price, plot_top, plot_bottom),
            ) else {
                continue;
            };
            let dist = point_segment_distance(cursor_x, cursor_y, ax, ay, bx, by);
            candidates.push((
                FigureTarget::TrendLine(idx),
                dist,
                clamp_x((ax + bx) / 2.0),
                clamp_y((ay + by) / 2.0),
            ));
        }

        let fibs = FIB_LINES.with(|state| state.borrow().clone());
        for (idx, (a, b)) in fibs.iter().enumerate() {
            let overlay = fib_overlay_from_anchors(*a, *b);
            let (Some(fx_start), Some(fx_end)) = (
                canvas_x_from_timestamp(overlay.x_start.clamp(view_x_start, view_x_end), plot_left, plot_right),
                canvas_x_from_timestamp(overlay.x_end.clamp(view_x_start, view_x_end), plot_left, plot_right),
            ) else {
                continue;
            };
            for (_ratio, price) in &overlay.levels {
                if !price.is_finite() || *price < y_low || *price > y_high {
                    continue;
                }
                let Some(ly) = canvas_y_from_price(*price, plot_top, plot_bottom) else {
                    continue;
                };
                let dist = point_segment_distance(cursor_x, cursor_y, fx_start, ly, fx_end, ly);
                candidates.push((FigureTarget::Fib(idx), dist, clamp_x(fx_end), clamp_y(ly)));
            }
        }

        if let (Some((mt_start, mt_end)), Some((p_start, p_end))) =
            (active_measure_range(), active_measure_price_range())
        {
            let xs = canvas_x_from_timestamp(mt_start.clamp(view_x_start, view_x_end), plot_left, plot_right);
            let xe = canvas_x_from_timestamp(mt_end.clamp(view_x_start, view_x_end), plot_left, plot_right);
            let ys = canvas_y_from_price(p_start, plot_top, plot_bottom);
            let ye = canvas_y_from_price(p_end, plot_top, plot_bottom);
            if let (Some(xs), Some(xe), Some(ys), Some(ye)) = (xs, xe, ys, ye) {
                // Distance to the price-range box edges.
                let edges = [
                    (xs, ys, xe, ys),
                    (xs, ye, xe, ye),
                    (xs, ys, xs, ye),
                    (xe, ys, xe, ye),
                ];
                let dist = edges
                    .into_iter()
                    .map(|(ax, ay, bx, by)| {
                        point_segment_distance(cursor_x, cursor_y, ax, ay, bx, by)
                    })
                    .fold(f64::INFINITY, f64::min);
                candidates.push((
                    FigureTarget::Measure,
                    dist,
                    clamp_x(xe),
                    clamp_y((ys + ye) / 2.0),
                ));
            }
        }

        candidates
            .into_iter()
            .filter(|(_, dist, _, _)| *dist <= TOL)
            .min_by(|(_, a, _, _), (_, b, _, _)| {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(target, _, x, y)| (target, x, y))
    }

    fn delete_figure(target: FigureTarget) {
        match target {
            FigureTarget::Fib(idx) => {
                FIB_LINES.with(|state| {
                    let mut fibs = state.borrow_mut();
                    if idx < fibs.len() {
                        fibs.remove(idx);
                    }
                });
            }
            FigureTarget::Measure => clear_measure(),
            FigureTarget::TrendLine(idx) => {
                TREND_LINES.with(|state| {
                    let mut lines = state.borrow_mut();
                    if idx < lines.len() {
                        lines.remove(idx);
                    }
                });
            }
        }
        persist_current_pair_drawings();
        sync_drawings_panel();
    }

    fn position_figure_trash(container_x: f64, container_y: f64) {
        if let Ok(doc) = document() {
            if let Some(el) = doc
                .get_element_by_id("figure-trash")
                .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            {
                let _ = el.style().set_property("display", "flex");
                let _ = el
                    .style()
                    .set_property("left", &format!("{}px", container_x.round() as i32));
                let _ = el
                    .style()
                    .set_property("top", &format!("{}px", container_y.round() as i32));
            }
        }
    }

    fn cancel_figure_trash_timer() {
        FIGURE_TRASH_HIDE_TIMER.with(|timer| {
            if let Some(id) = timer.borrow_mut().take() {
                if let Some(win) = web_sys::window() {
                    win.clear_timeout_with_handle(id);
                }
            }
        });
    }

    // Immediately hide the trash icon and forget its target (used during drags,
    // pans, active tools and when the cursor leaves the plot).
    fn hide_figure_trash() {
        cancel_figure_trash_timer();
        FIGURE_TRASH_TARGET.with(|target| {
            *target.borrow_mut() = None;
        });
        if let Ok(doc) = document() {
            if let Some(el) = doc
                .get_element_by_id("figure-trash")
                .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            {
                let _ = el.style().set_property("display", "none");
            }
        }
    }

    // Hide after a short grace period so the cursor can travel from the figure to
    // the icon without it vanishing. A no-op if a hide is already pending or no
    // figure is targeted.
    fn schedule_hide_figure_trash() {
        if FIGURE_TRASH_HIDE_TIMER.with(|timer| timer.borrow().is_some()) {
            return;
        }
        if FIGURE_TRASH_TARGET.with(|target| target.borrow().is_none()) {
            return;
        }
        FIGURE_TRASH_HIDE_CLOSURE.with(|closure| {
            if let Some(closure) = closure.borrow().as_ref() {
                if let Some(win) = web_sys::window() {
                    if let Ok(id) = win
                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                            closure.as_ref().unchecked_ref(),
                            180,
                        )
                    {
                        FIGURE_TRASH_HIDE_TIMER.with(|timer| {
                            *timer.borrow_mut() = Some(id);
                        });
                    }
                }
            }
        });
    }

    fn show_fib_popup() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("fib-popup") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "block");
                }
            }
        }
    }

    fn fib_popup_text_for_cursor(cursor_ts: i64, cursor_price: f64) -> String {
        FIB_STATE.with(|state| {
            let cfg = *state.borrow();
            if !cfg.enabled {
                return "Fib is off. Toggle Fib to start.".to_string();
            }

            match cfg.draft {
                None => format!(
                    "Fib is on. Cursor: {} @ {:.2}. Click first point.",
                    unix_seconds_to_hover_text(cursor_ts),
                    cursor_price
                ),
                Some((a_ts, a_price)) => format!(
                    "A: {} @ {:.2}. Cursor: {} @ {:.2}. Click second point.",
                    unix_seconds_to_hover_text(a_ts),
                    a_price,
                    unix_seconds_to_hover_text(cursor_ts),
                    cursor_price
                ),
            }
        })
    }

    fn hide_hover_tooltip() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("hover-tooltip") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn hide_cursor_time_label() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-time-label") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn hide_cursor_vline() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn show_cursor_vline(x: i32, plot_top: f64, plot_bottom: f64) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", x));
                    let _ = el
                        .style()
                        .set_property("top", &format!("{}px", plot_top.round() as i32));
                    let _ = el.style().set_property(
                        "height",
                        &format!("{}px", (plot_bottom - plot_top).max(0.0).round() as i32),
                    );
                }
            }
        }
    }

    // Position the floating last-price tag at the right edge of the plot, level
    // with `last_close`, and flash it green (up) or red (down) when the value
    // changes. Pass `None` to hide it (no candles / price off-screen).
    fn update_last_price_tag(last_close: Option<f64>) {
        let Ok(doc) = document() else {
            return;
        };
        let Some(node) = doc.get_element_by_id("last-price-tag") else {
            return;
        };
        let Ok(el) = node.dyn_into::<HtmlElement>() else {
            return;
        };

        let hide = |el: &HtmlElement| {
            let _ = el.style().set_property("display", "none");
            el.set_class_name("");
            LAST_PRICE_TAG_VALUE.with(|v| *v.borrow_mut() = None);
        };

        let Some(price) = last_close.filter(|p| p.is_finite()) else {
            hide(&el);
            return;
        };

        // Only show the tag while the price sits inside the visible band, matching
        // the on-canvas last-price line.
        let in_view = CHART_VIEW.with(|view| {
            view.borrow()
                .map(|cfg| price >= cfg.y_low && price <= cfg.y_high)
                .unwrap_or(false)
        });
        if !in_view {
            hide(&el);
            return;
        }

        let canvas = match doc
            .get_element_by_id("chart")
            .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
        {
            Some(c) => c,
            None => return,
        };
        let width = canvas.client_width() as f64;
        let height = canvas.client_height() as f64;
        let Some((_, plot_right, plot_top, plot_bottom)) = plot_bounds(width, height) else {
            hide(&el);
            return;
        };
        let Some(y) = canvas_y_from_price(price, plot_top, plot_bottom) else {
            hide(&el);
            return;
        };

        let canvas_rect = canvas.get_bounding_client_rect();
        let parent_rect = canvas
            .parent_element()
            .map(|p| p.get_bounding_client_rect());
        let canvas_left = parent_rect
            .as_ref()
            .map(|p| canvas_rect.left() - p.left())
            .unwrap_or(0.0);
        let canvas_top = parent_rect
            .as_ref()
            .map(|p| canvas_rect.top() - p.top())
            .unwrap_or(0.0);

        let style = el.style();
        let _ = style.set_property("display", "block");
        let _ = style.set_property(
            "left",
            &format!("{}px", (canvas_left + plot_right).round() as i32),
        );
        let _ = style.set_property("top", &format!("{}px", (canvas_top + y).round() as i32));
        el.set_text_content(Some(&format_price_label(price)));

        let prev = LAST_PRICE_TAG_VALUE.with(|v| *v.borrow());
        let direction = match prev {
            Some(p) if price > p => Some("flash-up"),
            Some(p) if price < p => Some("flash-down"),
            _ => None,
        };
        LAST_PRICE_TAG_VALUE.with(|v| *v.borrow_mut() = Some(price));

        if let Some(direction) = direction {
            // Restart the CSS animation: clear the class, force a reflow, re-apply.
            el.set_class_name("");
            let _ = el.offset_width();
            el.set_class_name(direction);
        }
    }

    fn hide_rsi_cursor_vline() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("rsi-cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn show_rsi_cursor_vline(x: i32) {
        if let Ok(doc) = document() {
            let main_canvas = match doc
                .get_element_by_id("chart")
                .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
            {
                Some(c) => c,
                None => return,
            };
            let rsi_canvas = match doc
                .get_element_by_id("rsi-chart")
                .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
            {
                Some(c) => c,
                None => return,
            };

            let rsi_canvas_rect = rsi_canvas.get_bounding_client_rect();
            let rsi_parent_rect = rsi_canvas
                .parent_element()
                .map(|el| el.get_bounding_client_rect());
            let rsi_canvas_left = rsi_parent_rect
                .as_ref()
                .map(|parent| rsi_canvas_rect.left() - parent.left())
                .unwrap_or(0.0);
            let rsi_canvas_top = rsi_parent_rect
                .as_ref()
                .map(|parent| rsi_canvas_rect.top() - parent.top())
                .unwrap_or(0.0);

            let main_width = main_canvas.client_width() as f64;
            let main_margin = 16.0;
            let main_y_label_area = 72.0;
            let main_plot_left = main_margin + main_y_label_area;
            let main_plot_right = main_width - main_margin;
            if main_plot_right <= main_plot_left {
                return;
            }

            let width = rsi_canvas.client_width() as f64;
            let height = rsi_canvas.client_height() as f64;
            let margin = 10.0;
            let y_label_area = 44.0;
            let x_label_area = 22.0;
            let plot_left = margin + y_label_area;
            let plot_right = width - margin;
            let plot_top = margin;
            let plot_bottom = height - margin - x_label_area;
            if plot_right <= plot_left || plot_bottom <= plot_top {
                return;
            }

            if let Some(node) = doc.get_element_by_id("rsi-cursor-vline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let main_ratio = (((x as f64).clamp(main_plot_left, main_plot_right)
                        - main_plot_left)
                        / (main_plot_right - main_plot_left))
                        .clamp(0.0, 1.0);
                    let mapped_x = plot_left + main_ratio * (plot_right - plot_left);
                    let clamped_x = (rsi_canvas_left + mapped_x).round() as i32;
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", clamped_x));
                    let _ = el.style().set_property(
                        "top",
                        &format!("{}px", (rsi_canvas_top + plot_top).round() as i32),
                    );
                    let _ = el.style().set_property(
                        "height",
                        &format!("{}px", (plot_bottom - plot_top).max(0.0).round() as i32),
                    );
                }
            }
        }
    }

    fn hide_cursor_hline() {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-hline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "none");
                }
            }
        }
    }

    fn show_cursor_hline(y: i32, plot_left: f64, plot_right: f64) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-hline") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("top", &format!("{}px", y));
                    let _ = el
                        .style()
                        .set_property("left", &format!("{}px", plot_left.round() as i32));
                    let _ = el.style().set_property(
                        "width",
                        &format!("{}px", (plot_right - plot_left).max(0.0).round() as i32),
                    );
                }
            }
        }
    }

    fn show_cursor_time_label(text: &str, x: i32) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("cursor-time-label") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    el.set_text_content(Some(text));
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", x));
                }
            }
        }
    }

    fn set_chart_cursor(cursor: &str) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("chart") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    let _ = el.style().set_property("cursor", cursor);
                }
            }
        }
    }

    fn show_hover_tooltip(text: &str, x: i32, y: i32) {
        if let Ok(doc) = document() {
            if let Some(node) = doc.get_element_by_id("hover-tooltip") {
                if let Ok(el) = node.dyn_into::<HtmlElement>() {
                    el.set_text_content(Some(text));
                    let _ = el.style().set_property("display", "block");
                    let _ = el.style().set_property("left", &format!("{}px", x + 14));
                    let _ = el.style().set_property("top", &format!("{}px", y + 14));
                }
            }
        }
    }

    fn set_settings_visible(visible: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let body = doc
            .get_element_by_id("settings-body")
            .ok_or_else(|| JsValue::from_str("missing settings body"))?;
        let toggle = doc
            .get_element_by_id("settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing settings toggle"))?;

        if visible {
            body.set_class_name("ma-settings-body settings-body");
            toggle.set_text_content(Some("Hide"));
        } else {
            body.set_class_name("ma-settings-body settings-body hidden");
            toggle.set_text_content(Some("Show"));
        }

        storage()?.set_item(
            STORAGE_KEY_SETTINGS_VISIBLE,
            if visible { "1" } else { "0" },
        )?;
        Ok(())
    }

    fn set_settings_side(side: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("ma-settings-card")
            .ok_or_else(|| JsValue::from_str("missing settings card"))?
            .dyn_into::<HtmlElement>()?;
        let side_toggle = doc.get_element_by_id("settings-side-toggle");

        if side == "left" {
            card.set_class_name("panel ma-settings-card left");
            if let Some(side_toggle) = &side_toggle {
                side_toggle.set_text_content(Some("Move Right"));
            }
            storage()?.set_item(STORAGE_KEY_SETTINGS_SIDE, "left")?;
        } else {
            card.set_class_name("panel ma-settings-card");
            if let Some(side_toggle) = &side_toggle {
                side_toggle.set_text_content(Some("Move Left"));
            }
            storage()?.set_item(STORAGE_KEY_SETTINGS_SIDE, "right")?;
        }
        let _ = card.style().remove_property("left");
        let _ = card.style().remove_property("top");
        let _ = card.style().remove_property("right");
        let _ = card.style().remove_property("bottom");
        Ok(())
    }

    fn settings_side() -> Result<String, JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("ma-settings-card")
            .ok_or_else(|| JsValue::from_str("missing settings card"))?;
        if card.class_name().contains(" left") {
            Ok("left".to_string())
        } else {
            Ok("right".to_string())
        }
    }

    fn settings_visible() -> Result<bool, JsValue> {
        let doc = document()?;
        let body = doc
            .get_element_by_id("settings-body")
            .ok_or_else(|| JsValue::from_str("missing settings body"))?;
        Ok(!body.class_name().contains("hidden"))
    }

    fn connection_settings_class_name(side: &str, collapsed: bool) -> &'static str {
        match (side == "left", collapsed) {
            (true, true) => "panel connection-settings-card left collapsed",
            (true, false) => "panel connection-settings-card left",
            (false, true) => "panel connection-settings-card collapsed",
            (false, false) => "panel connection-settings-card",
        }
    }

    fn set_connection_settings_visible(visible: bool) -> Result<(), JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?;
        let body = doc
            .get_element_by_id("connection-settings-body")
            .ok_or_else(|| JsValue::from_str("missing connection settings body"))?;
        let toggle = doc
            .get_element_by_id("connection-settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing connection settings toggle"))?;

        if visible {
            body.set_class_name("connection-settings-body");
            toggle.set_text_content(Some("Hide"));
            let side = if card.class_name().contains(" left") {
                "left"
            } else {
                "right"
            };
            card.set_class_name(connection_settings_class_name(side, false));
        } else {
            body.set_class_name("connection-settings-body hidden");
            toggle.set_text_content(Some("Show"));
            let side = if card.class_name().contains(" left") {
                "left"
            } else {
                "right"
            };
            card.set_class_name(connection_settings_class_name(side, true));
        }

        storage()?.set_item(
            STORAGE_KEY_CONNECTION_SETTINGS_VISIBLE,
            if visible { "1" } else { "0" },
        )?;
        Ok(())
    }

    fn set_connection_settings_side(side: &str) -> Result<(), JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?
            .dyn_into::<HtmlElement>()?;
        let side_toggle = doc.get_element_by_id("connection-settings-side-toggle");

        let collapsed = card.class_name().contains(" collapsed");

        if side == "left" {
            card.set_class_name(connection_settings_class_name("left", collapsed));
            if let Some(side_toggle) = &side_toggle {
                side_toggle.set_text_content(Some("Move Right"));
            }
            storage()?.set_item(STORAGE_KEY_CONNECTION_SETTINGS_SIDE, "left")?;
        } else {
            card.set_class_name(connection_settings_class_name("right", collapsed));
            if let Some(side_toggle) = &side_toggle {
                side_toggle.set_text_content(Some("Move Left"));
            }
            storage()?.set_item(STORAGE_KEY_CONNECTION_SETTINGS_SIDE, "right")?;
        }
        let _ = card.style().remove_property("left");
        let _ = card.style().remove_property("top");
        let _ = card.style().remove_property("right");
        let _ = card.style().remove_property("bottom");
        Ok(())
    }

    fn connection_settings_side() -> Result<String, JsValue> {
        let doc = document()?;
        let card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?;
        if card.class_name().contains(" left") {
            Ok("left".to_string())
        } else {
            Ok("right".to_string())
        }
    }

    fn connection_settings_visible() -> Result<bool, JsValue> {
        let doc = document()?;
        let body = doc
            .get_element_by_id("connection-settings-body")
            .ok_or_else(|| JsValue::from_str("missing connection settings body"))?;
        Ok(!body.class_name().contains("hidden"))
    }

    fn storage() -> Result<Storage, JsValue> {
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window is not available"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("localStorage is not available"))
    }

    fn save_inputs() -> Result<(), JsValue> {
        let storage = storage()?;
        storage.set_item(STORAGE_KEY_API_BASE, &input_value("api-base")?)?;
        storage.set_item(STORAGE_KEY_DB, &select_value("db")?)?;
        storage.set_item(STORAGE_KEY_CHART_SOURCE, &select_value("chart-source")?)?;
        storage.set_item(STORAGE_KEY_PERIOD, &input_value("period")?)?;
        storage.set_item(STORAGE_KEY_TS_START, &input_value("ts-start-human")?)?;
        storage.set_item(STORAGE_KEY_TS_END, &input_value("ts-end-human")?)?;
        storage.set_item(
            STORAGE_KEY_LOG_SCALE,
            if checkbox_checked("log-scale")? {
                "1"
            } else {
                "0"
            },
        )?;
        for idx in 1..=MA_COUNT {
            let enabled_key = format!("price_api.ma{idx}.enabled");
            let period_key = format!("price_api.ma{idx}.period");
            storage.set_item(
                &enabled_key,
                if checkbox_checked(&ma_enabled_id(idx))? {
                    "1"
                } else {
                    "0"
                },
            )?;
            storage.set_item(&period_key, &input_value(&ma_period_id(idx))?)?;
        }
        Ok(())
    }

    fn load_saved_inputs() -> Result<(), JsValue> {
        let storage = storage()?;

        if let Some(v) = storage.get_item(STORAGE_KEY_API_BASE)? {
            if !v.is_empty() {
                set_input_value("api-base", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_DB)? {
            if !v.is_empty() {
                set_select_value("db", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_CHART_SOURCE)? {
            if !v.is_empty() {
                set_select_value("chart-source", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_PERIOD)? {
            if !v.is_empty() {
                set_input_value("period", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_TS_START)? {
            if !v.is_empty() {
                set_input_value("ts-start-human", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_TS_END)? {
            if !v.is_empty() {
                set_input_value("ts-end-human", &v)?;
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_LOG_SCALE)? {
            set_checkbox_checked("log-scale", v == "1")?;
        }
        sync_log_scale_button()?;
        if let Some(v) = storage.get_item(STORAGE_KEY_CHART_FLIPPED)? {
            CHART_FLIPPED.with(|state| *state.borrow_mut() = v == "1");
        }
        sync_flip_button()?;
        for idx in 1..=MA_COUNT {
            let enabled_key = format!("price_api.ma{idx}.enabled");
            let period_key = format!("price_api.ma{idx}.period");
            if let Some(v) = storage.get_item(&enabled_key)? {
                set_checkbox_checked(&ma_enabled_id(idx), v == "1")?;
            }
            if let Some(v) = storage.get_item(&period_key)? {
                if !v.is_empty() {
                    set_input_value(&ma_period_id(idx), &v)?;
                }
            }
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_SETTINGS_VISIBLE)? {
            set_settings_visible(v == "1")?;
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_SETTINGS_SIDE)? {
            set_settings_side(&v)?;
        } else {
            set_settings_side("left")?;
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_CONNECTION_SETTINGS_VISIBLE)? {
            set_connection_settings_visible(v == "1")?;
        } else {
            set_connection_settings_visible(true)?;
        }
        if let Some(v) = storage.get_item(STORAGE_KEY_CONNECTION_SETTINGS_SIDE)? {
            set_connection_settings_side(&v)?;
        } else {
            set_connection_settings_side("left")?;
        }

        Ok(())
    }

    fn datetime_local_to_unix_seconds(value: &str) -> Result<i64, JsValue> {
        let date = Date::new(&JsValue::from_str(value));
        let millis = date.get_time();
        if millis.is_nan() {
            return Err(JsValue::from_str("invalid datetime value"));
        }
        Ok((millis / 1000.0) as i64)
    }

    fn unix_seconds_to_datetime_local(ts: i64) -> String {
        let d = Date::new(&JsValue::from_f64((ts * 1000) as f64));
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date(),
            d.get_hours(),
            d.get_minutes()
        )
    }

    fn unix_seconds_to_hover_text(ts: i64) -> String {
        let d = Date::new(&JsValue::from_f64((ts * 1000) as f64));
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date(),
            d.get_hours(),
            d.get_minutes()
        )
    }

    fn y_tick_values(y_low: f64, y_high: f64, use_log: bool, count: usize) -> Vec<f64> {
        if count == 0 || !y_low.is_finite() || !y_high.is_finite() || y_high <= y_low {
            return Vec::new();
        }
        let n = count as f64;
        let mut out = Vec::with_capacity(count + 1);
        let can_log = use_log && y_low > 0.0;
        let (a, b) = if can_log {
            (y_low.ln(), y_high.ln())
        } else {
            (y_low, y_high)
        };
        for i in 0..=count {
            let t = i as f64 / n;
            let v = a + (b - a) * t;
            out.push(if can_log { v.exp() } else { v });
        }
        out
    }

    fn format_price_label(y: f64) -> String {
        let abs = y.abs();
        if abs >= 1000.0 {
            format!("{:.0}", y)
        } else if abs >= 10.0 {
            format!("{:.2}", y)
        } else if abs >= 1.0 {
            format!("{:.3}", y)
        } else if abs >= 0.01 {
            format!("{:.4}", y)
        } else if abs > 0.0 {
            format!("{:.6}", y)
        } else {
            "0".to_string()
        }
    }

    fn format_measure_price_label(start: f64, end: f64) -> String {
        let delta = end - start;
        if start.abs() > f64::EPSILON && start.is_finite() && end.is_finite() {
            let percent = (end / start - 1.0) * 100.0;
            format!("{} ({:+.2}%)", format_price_label(delta), percent)
        } else {
            format_price_label(delta)
        }
    }

    fn unix_seconds_to_date_text(ts: i64) -> String {
        let d = Date::new(&JsValue::from_f64((ts * 1000) as f64));
        format!(
            "{:04}-{:02}-{:02}",
            d.get_full_year() as i32,
            d.get_month() + 1,
            d.get_date()
        )
    }

    enum ChartRequest {
        Direct(String),
        Ratio { num_url: String, den_url: String },
    }

    fn chart_source_pairs(
        source: &str,
    ) -> Result<
        (
            (&'static str, &'static str),
            Option<(&'static str, &'static str)>,
        ),
        JsValue,
    > {
        match source {
            "btc_usd" => Ok((("btc", "usd"), None)),
            "eth_usd" => Ok((("eth", "usd"), None)),
            "sol_usd" => Ok((("sol", "usd"), None)),
            "xau_usd" => Ok((("xau", "usd"), None)),
            "eth_btc" => Ok((("eth", "usd"), Some(("btc", "usd")))),
            "sol_btc" => Ok((("sol", "btc"), None)),
            "btc_xau" => Ok((("btc", "usd"), Some(("xau", "usd")))),
            "sol_eth" => Ok((("sol", "usd"), Some(("eth", "usd")))),
            _ => Err(JsValue::from_str(
                "chart-source must be btc_usd, eth_usd, sol_usd, xau_usd, eth_btc, sol_btc, btc_xau, or sol_eth",
            )),
        }
    }

    fn build_candle_url(
        api_base: &str,
        db: &str,
        pair: (&str, &str),
        period: &str,
        ts_start: i64,
        ts_end: i64,
    ) -> String {
        format!(
            "{base}/candles/{db}/{b}/{q}?period={period}&ts_start={ts_start}&ts_end={ts_end}",
            base = api_base.trim_end_matches('/'),
            b = pair.0,
            q = pair.1,
        )
    }

    fn build_request() -> Result<ChartRequest, JsValue> {
        let api_base = input_value("api-base")?;
        let db = select_value("db")?;
        let chart_source = select_value("chart-source")?;
        let period = input_value("period")?;
        let ts_start_human = input_value("ts-start-human")?;
        let ts_end_human = input_value("ts-end-human")?;

        let ts_start = datetime_local_to_unix_seconds(&ts_start_human)?;
        let ts_end = datetime_local_to_unix_seconds(&ts_end_human)?;
        let (num, den) = chart_source_pairs(&chart_source)?;

        let num_url = build_candle_url(&api_base, &db, num, &period, ts_start, ts_end);
        match den {
            None => Ok(ChartRequest::Direct(num_url)),
            Some(den_pair) => {
                let den_url = build_candle_url(&api_base, &db, den_pair, &period, ts_start, ts_end);
                Ok(ChartRequest::Ratio { num_url, den_url })
            }
        }
    }

    fn compute_ratio_candles(num: &[Candle], den: &[Candle]) -> Vec<Candle> {
        let mut out = Vec::with_capacity(num.len().min(den.len()));
        let (mut i, mut j) = (0usize, 0usize);
        while i < num.len() && j < den.len() {
            let n = &num[i];
            let d = &den[j];
            if n.timestamp < d.timestamp {
                i += 1;
            } else if n.timestamp > d.timestamp {
                j += 1;
            } else {
                if d.open > 0.0 && d.close > 0.0 && d.high > 0.0 && d.low > 0.0 {
                    out.push(Candle {
                        timestamp: n.timestamp,
                        open: n.open / d.open,
                        close: n.close / d.close,
                        high: n.high / d.low,
                        low: n.low / d.high,
                        volume: 0.0,
                    });
                }
                i += 1;
                j += 1;
            }
        }
        out
    }

    fn build_websocket_url(db: &str) -> Result<String, JsValue> {
        let api_base = input_value("api-base")?;
        let base = api_base.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(JsValue::from_str("api-base is empty"));
        }

        let ws_base = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else if base.starts_with("wss://") || base.starts_with("ws://") {
            base.to_string()
        } else {
            return Err(JsValue::from_str(
                "api-base must start with http:// or https://",
            ));
        };

        Ok(format!("{ws_base}/ws/{db}"))
    }

    fn disconnect_realtime_ws() {
        LIVE_WS.with(|state| {
            if let Some(conn) = state.borrow_mut().take() {
                conn.ws.set_onopen(None);
                conn.ws.set_onmessage(None);
                conn.ws.set_onerror(None);
                conn.ws.set_onclose(None);
                let _ = conn.ws.close();
            }
        });
    }

    // Fold a freshly inserted candle into LAST_CANDLES. Within the current
    // aggregation bucket we extend high/low, carry the latest close and accumulate
    // volume; once the candle crosses into a new bucket we append a fresh one
    // aligned to the inferred spacing.
    fn merge_live_candle(new: &Candle) {
        LAST_CANDLES.with(|state| {
            let mut candles = state.borrow_mut();
            let spacing = inferred_candle_spacing(&candles);
            match candles.last_mut() {
                Some(last) if new.timestamp < last.timestamp => {
                    // Stale or out-of-order candle; ignore.
                }
                Some(last) if new.timestamp < last.timestamp + spacing => {
                    last.high = last.high.max(new.high);
                    last.low = last.low.min(new.low);
                    last.close = new.close;
                    last.volume += new.volume;
                }
                Some(last) => {
                    let bucket_ts =
                        last.timestamp + ((new.timestamp - last.timestamp) / spacing) * spacing;
                    candles.push(Candle {
                        timestamp: bucket_ts,
                        open: new.open,
                        high: new.high,
                        low: new.low,
                        close: new.close,
                        volume: new.volume,
                    });
                }
                None => candles.push(new.clone()),
            }
        });
    }

    // Merge a live candle into the cached series and redraw, keeping the live edge
    // in view if the chart was already following it.
    async fn apply_live_candle_and_render(new: Candle) -> Result<(), JsValue> {
        let prev_last_ts = LAST_CANDLES.with(|state| state.borrow().last().map(|c| c.timestamp));
        merge_live_candle(&new);
        let new_last_ts = LAST_CANDLES.with(|state| state.borrow().last().map(|c| c.timestamp));

        // If a new bucket opened and the view was pinned to the old live edge,
        // advance the view so the latest candle stays visible.
        if let (Some(prev), Some(latest)) = (prev_last_ts, new_last_ts) {
            if latest > prev {
                if let Some((view_start, view_end)) = rendered_range() {
                    if view_end >= prev {
                        let span = (view_end - view_start).max(60);
                        let next_end = latest;
                        let next_start = next_end - span;
                        CLIENT_VIEW_RANGE.with(|state| {
                            *state.borrow_mut() = Some((next_start, next_end));
                        });
                        save_view_range(next_start, next_end);
                    }
                }
            }
        }

        rerender_cached_or_fetch().await
    }

    fn connect_realtime_ws() -> Result<(), JsValue> {
        disconnect_realtime_ws();

        let db = select_value("db")?;
        let url = build_websocket_url(&db)?;
        let ws = WebSocket::new(&url)?;

        let subscribed_db = db.clone();
        let onopen = Closure::wrap(Box::new(move |_event: Event| {}) as Box<dyn FnMut(Event)>);
        let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            let Ok(update) = serde_json::from_str::<RealtimeUpdateEvent>(&text) else {
                return;
            };
            // Only react to updates for the database this socket subscribed to and
            // that is still the active selection.
            if update.db != subscribed_db {
                return;
            }
            if select_value("db").ok().as_deref() != Some(subscribed_db.as_str()) {
                return;
            }
            // Only react to the pair(s) feeding the active chart source.
            let Ok(chart_source) = select_value("chart-source") else {
                return;
            };
            let Ok((num, den)) = chart_source_pairs(&chart_source) else {
                return;
            };
            let event_pair = (update.base.as_str(), update.quote.as_str());
            let matches_num = event_pair == num;
            let matches_den = den.map(|d| event_pair == d).unwrap_or(false);
            if !matches_num && !matches_den {
                return;
            }
            if den.is_some() {
                // Ratio charts combine two pairs, so a single-pair update can't be
                // merged locally; refetch both legs to recompute the ratio.
                spawn_local(async {
                    if let Err(err) = fetch_and_draw().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            } else {
                let candle = update.candle.clone();
                spawn_local(async move {
                    if let Err(err) = apply_live_candle_and_render(candle).await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        let onerror = Closure::wrap(Box::new(move |_event: Event| {}) as Box<dyn FnMut(Event)>);
        let onclose = Closure::wrap(Box::new(move |_event: Event| {}) as Box<dyn FnMut(Event)>);

        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

        LIVE_WS.with(|state| {
            *state.borrow_mut() = Some(LiveWsConnection {
                ws,
                _onopen: onopen,
                _onmessage: onmessage,
                _onerror: onerror,
                _onclose: onclose,
            });
        });

        Ok(())
    }

    fn ma_enabled_id(idx: usize) -> String {
        format!("ma{idx}-enabled")
    }

    fn ma_period_id(idx: usize) -> String {
        format!("ma{idx}-period")
    }

    fn ma_color(idx: usize) -> RGBColor {
        match idx {
            1 => RGBColor(33, 74, 159),
            2 => RGBColor(159, 99, 33),
            3 => RGBColor(129, 47, 153),
            4 => RGBColor(48, 138, 124),
            5 => RGBColor(194, 64, 46),
            6 => RGBColor(76, 102, 18),
            _ => RGBColor(126, 54, 89),
        }
    }

    fn ma_default_period(idx: usize) -> usize {
        match idx {
            1 => 13,
            2 => 21,
            3 => 34,
            4 => 55,
            5 => 89,
            6 => 144,
            7 => 233,
            8 => 377,
            9 => 610,
            10 => 987,
            11 => 1597,
            12 => 2584,
            13 => 4181,
            14 => 6765,
            _ => 10946,
        }
    }

    fn moving_average_configs() -> Result<Vec<MovingAverageConfig>, JsValue> {
        let mut configs = Vec::with_capacity(MA_COUNT);
        for idx in 1..=MA_COUNT {
            let enabled = checkbox_checked(&ma_enabled_id(idx))?;
            let period = input_value(&ma_period_id(idx))?
                .parse::<usize>()
                .unwrap_or(ma_default_period(idx))
                .max(2);
            configs.push(MovingAverageConfig {
                idx,
                enabled,
                period,
                color: ma_color(idx),
            });
        }
        Ok(configs)
    }

    fn sma_points(candles: &[Candle], window: usize) -> Vec<(i64, f64)> {
        if candles.len() < window {
            return Vec::new();
        }

        let mut points = Vec::with_capacity(candles.len() - window + 1);
        let mut rolling_sum = 0.0;

        for (idx, candle) in candles.iter().enumerate() {
            rolling_sum += candle.close;
            if idx >= window {
                rolling_sum -= candles[idx - window].close;
            }
            if idx + 1 >= window {
                points.push((candle.timestamp, rolling_sum / window as f64));
            }
        }

        points
    }

    /// Keep only the moving-average points needed to draw the line across the
    /// visible window: everything inside `[x_start, x_end]` plus the single
    /// point just outside each edge so the line still reaches the borders.
    fn clip_points_to_range(points: Vec<(i64, f64)>, x_start: i64, x_end: i64) -> Vec<(i64, f64)> {
        if points.is_empty() {
            return points;
        }
        let first_inside = points.iter().position(|(ts, _)| *ts >= x_start);
        let last_inside = points.iter().rposition(|(ts, _)| *ts <= x_end);
        let (Some(first_inside), Some(last_inside)) = (first_inside, last_inside) else {
            return Vec::new();
        };
        if first_inside > last_inside {
            return Vec::new();
        }
        let lo = first_inside.saturating_sub(1);
        let hi = (last_inside + 1).min(points.len() - 1);
        points[lo..=hi].to_vec()
    }

    fn rsi_points(candles: &[Candle], period: usize) -> Vec<(i64, f64)> {
        if candles.len() <= period {
            return Vec::new();
        }

        let mut gains = 0.0;
        let mut losses = 0.0;
        for i in 1..=period {
            let diff = candles[i].close - candles[i - 1].close;
            if diff >= 0.0 {
                gains += diff;
            } else {
                losses += -diff;
            }
        }

        let mut avg_gain = gains / period as f64;
        let mut avg_loss = losses / period as f64;
        let mut points = Vec::with_capacity(candles.len() - period);

        let first_rsi = if avg_loss == 0.0 {
            100.0
        } else {
            let rs = avg_gain / avg_loss;
            100.0 - (100.0 / (1.0 + rs))
        };
        points.push((candles[period].timestamp, first_rsi));

        for i in (period + 1)..candles.len() {
            let diff = candles[i].close - candles[i - 1].close;
            let gain = diff.max(0.0);
            let loss = (-diff).max(0.0);

            avg_gain = ((avg_gain * (period as f64 - 1.0)) + gain) / period as f64;
            avg_loss = ((avg_loss * (period as f64 - 1.0)) + loss) / period as f64;

            let rsi = if avg_loss == 0.0 {
                100.0
            } else {
                let rs = avg_gain / avg_loss;
                100.0 - (100.0 / (1.0 + rs))
            };

            points.push((candles[i].timestamp, rsi));
        }

        points
    }

    fn draw_rsi(candles: &[Candle]) -> Result<(), JsValue> {
        let doc = document()?;
        let canvas = doc
            .get_element_by_id("rsi-chart")
            .ok_or_else(|| JsValue::from_str("missing rsi chart canvas"))?
            .dyn_into::<HtmlCanvasElement>()?;

        sync_canvas_backing_size(&canvas);
        let backend = CanvasBackend::with_canvas_object(canvas)
            .ok_or_else(|| JsValue::from_str("rsi canvas backend error"))?;
        let root = backend.into_drawing_area();
        root.fill(&RGBColor(246, 247, 251))
            .map_err(|e| JsValue::from_str(&format!("rsi background error: {e}")))?;

        let (x_start, x_end) = rendered_range()
            .or_else(|| {
                candles.first().zip(candles.last()).map(|(first, last)| {
                    let start = first.timestamp.min(last.timestamp);
                    let end = first.timestamp.max(last.timestamp);
                    (start, (start + 60).max(end))
                })
            })
            .unwrap_or((0, 60));

        if candles.is_empty() && LAST_CANDLES.with(|state| state.borrow().is_empty()) {
            root.present()
                .map_err(|e| JsValue::from_str(&format!("rsi present error: {e}")))?;
            return Ok(());
        }

        let mut chart = ChartBuilder::on(&root)
            .margin(10)
            .x_label_area_size(22)
            .y_label_area_size(44)
            .caption("RSI (14)", ("sans-serif", 16).into_font())
            .build_cartesian_2d(x_start..x_end, 0.0f64..100.0f64)
            .map_err(|e| JsValue::from_str(&format!("rsi chart build error: {e}")))?;

        chart
            .configure_mesh()
            .x_labels(8)
            .y_labels(5)
            .disable_x_mesh()
            .x_label_formatter(&|x| unix_seconds_to_date_text(*x))
            .draw()
            .map_err(|e| JsValue::from_str(&format!("rsi mesh draw error: {e}")))?;

        let points = rsi_points(candles, 14);
        if !points.is_empty() {
            chart
                .draw_series(LineSeries::new(points, &RGBColor(24, 96, 173)))
                .map_err(|e| JsValue::from_str(&format!("rsi line draw error: {e}")))?;
        }

        chart
            .draw_series(LineSeries::new(
                vec![(x_start, 70.0), (x_end, 70.0)],
                &RGBColor(176, 78, 66),
            ))
            .map_err(|e| JsValue::from_str(&format!("rsi 70 draw error: {e}")))?;
        chart
            .draw_series(LineSeries::new(
                vec![(x_start, 30.0), (x_end, 30.0)],
                &RGBColor(59, 138, 101),
            ))
            .map_err(|e| JsValue::from_str(&format!("rsi 30 draw error: {e}")))?;

        root.present()
            .map_err(|e| JsValue::from_str(&format!("rsi present error: {e}")))?;

        Ok(())
    }

    fn selected_ts_range() -> Result<(i64, i64), JsValue> {
        let ts_start_human = input_value("ts-start-human")?;
        let ts_end_human = input_value("ts-end-human")?;
        let ts_start = datetime_local_to_unix_seconds(&ts_start_human)?;
        let ts_end = datetime_local_to_unix_seconds(&ts_end_human)?;
        Ok((ts_start, ts_end))
    }

    fn set_ts_range(ts_start: i64, ts_end: i64) -> Result<(), JsValue> {
        set_input_value("ts-start-human", &unix_seconds_to_datetime_local(ts_start))?;
        set_input_value("ts-end-human", &unix_seconds_to_datetime_local(ts_end))?;
        Ok(())
    }

    fn zoomed_range(factor: f64) -> Result<(i64, i64), JsValue> {
        let (ts_start, ts_end) = selected_ts_range()?;
        let span = (ts_end - ts_start).max(60);
        let center = ts_start + span / 2;
        let new_span = ((span as f64) * factor).round() as i64;
        let clamped_span = new_span.max(60);
        let new_start = center - clamped_span / 2;
        let new_end = center + clamped_span / 2;
        Ok((new_start, new_end))
    }

    fn panned_range(direction: i64) -> Result<(i64, i64), JsValue> {
        let (ts_start, ts_end) = selected_ts_range()?;
        let span = (ts_end - ts_start).max(60);
        let step = ((span as f64) * 0.25).round() as i64;
        let shift = step.max(60) * direction;
        Ok((ts_start + shift, ts_end + shift))
    }

    fn zoomed_range_from(ts_start: i64, ts_end: i64, factor: f64) -> (i64, i64) {
        let span = (ts_end - ts_start).max(60);
        let center = ts_start + span / 2;
        let new_span = ((span as f64) * factor).round() as i64;
        let clamped_span = new_span.max(60);
        let new_start = center - clamped_span / 2;
        let new_end = center + clamped_span / 2;
        (new_start, new_end)
    }

    fn panned_range_from(ts_start: i64, ts_end: i64, direction: i64) -> (i64, i64) {
        let span = (ts_end - ts_start).max(60);
        let step = ((span as f64) * 0.10).round() as i64;
        let shift = step.max(60) * direction;
        (ts_start + shift, ts_end + shift)
    }

    fn save_view_range(ts_start: i64, ts_end: i64) {
        let Ok(storage) = storage() else {
            return;
        };
        let Ok(period) = input_value("period") else {
            return;
        };
        let _ = storage.set_item(STORAGE_KEY_VIEW_START, &ts_start.to_string());
        let _ = storage.set_item(STORAGE_KEY_VIEW_END, &ts_end.to_string());
        let _ = storage.set_item(STORAGE_KEY_VIEW_PERIOD, &period);
    }

    // Returns the persisted zoom/pan view range only when it was saved for the
    // currently selected period (timeframe), so a reload keeps the same view.
    fn saved_view_range_for_current_period() -> Option<(i64, i64)> {
        let storage = storage().ok()?;
        let saved_period = storage.get_item(STORAGE_KEY_VIEW_PERIOD).ok()??;
        let current_period = input_value("period").ok()?;
        if saved_period != current_period {
            return None;
        }
        let start = storage
            .get_item(STORAGE_KEY_VIEW_START)
            .ok()??
            .parse::<i64>()
            .ok()?;
        let end = storage
            .get_item(STORAGE_KEY_VIEW_END)
            .ok()??
            .parse::<i64>()
            .ok()?;
        Some((start, end))
    }

    fn rendered_range() -> Option<(i64, i64)> {
        CLIENT_VIEW_RANGE.with(|state| *state.borrow()).or_else(|| {
            LAST_RENDERED_CANDLES.with(|state| {
                let candles = state.borrow();
                let first = candles.first()?.timestamp;
                let last = candles.last()?.timestamp;
                Some((first.min(last), first.max(last)))
            })
        })
    }

    fn loaded_bounds() -> Option<(i64, i64)> {
        LAST_CANDLES.with(|state| {
            let candles = state.borrow();
            let first = candles.first()?.timestamp;
            let last = candles.last()?.timestamp;
            Some((first.min(last), first.max(last)))
        })
    }

    fn candle_bounds(candles: &[Candle]) -> Option<(i64, i64)> {
        let first = candles.first()?.timestamp;
        let last = candles.last()?.timestamp;
        Some((first.min(last), first.max(last)))
    }

    fn loaded_candle_spacing() -> i64 {
        LAST_CANDLES.with(|state| inferred_candle_spacing(&state.borrow()))
    }

    fn clamp_range_to_loaded(ts_start: i64, ts_end: i64) -> (i64, i64) {
        let (mut start, mut end) = if ts_start <= ts_end {
            (ts_start, ts_end)
        } else {
            (ts_end, ts_start)
        };
        let mut span = (end - start).max(60);

        if let Some((min_ts, _max_ts)) = loaded_bounds() {
            start = start.max(min_ts);
            end = start + span;
        } else {
            end = start + span;
        }

        if end <= start {
            end = start + 60;
        }

        (start, end)
    }

    fn filter_candles_by_range(candles: &[Candle], ts_start: i64, ts_end: i64) -> Vec<Candle> {
        candles
            .iter()
            .filter(|c| c.timestamp >= ts_start && c.timestamp <= ts_end)
            .cloned()
            .collect()
    }

    fn apply_range_change_and_fetch(ts_start: i64, ts_end: i64) {
        let (new_start, new_end) = clamp_range_to_loaded(ts_start, ts_end);

        if let Ok((old_start, old_end)) = selected_ts_range() {
            if old_start == new_start && old_end == new_end {
                return;
            }
            RANGE_HISTORY.with(|history| {
                history.borrow_mut().push((old_start, old_end));
            });
        }

        if let Err(err) = set_ts_range(new_start, new_end) {
            set_status(&format!("failed to apply frame: {:?}", err));
            return;
        }

        spawn_local(async {
            if let Err(err) = rerender_cached_or_fetch().await {
                set_status(&format!("failed: {:?}", err));
            }
        });
    }

    fn undo_last_range_change() {
        let previous = RANGE_HISTORY.with(|history| history.borrow_mut().pop());
        if let Some((ts_start, ts_end)) = previous {
            let (new_start, new_end) = clamp_range_to_loaded(ts_start, ts_end);
            if let Err(err) = set_ts_range(new_start, new_end) {
                set_status(&format!("failed undo: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        } else {
            set_status("Nothing to undo");
        }
    }

    fn apply_range_change_client_only(ts_start: i64, ts_end: i64) -> Result<(), JsValue> {
        if AUTO_MODE_ENABLED.with(|s| *s.borrow()) {
            Y_STRETCH_FACTOR.with(|s| *s.borrow_mut() = 1.0);
            Y_PAN_LINEAR_OFFSET.with(|s| *s.borrow_mut() = 0.0);
            Y_PAN_LOG_OFFSET.with(|s| *s.borrow_mut() = 0.0);
        }
        let (new_start, new_end) = clamp_range_to_loaded(ts_start, ts_end);
        CLIENT_VIEW_RANGE.with(|state| {
            *state.borrow_mut() = Some((new_start, new_end));
        });
        save_view_range(new_start, new_end);

        let candles = LAST_CANDLES.with(|state| state.borrow().clone());
        if candles.is_empty() {
            return Ok(());
        }

        let visible = filter_candles_by_range(&candles, new_start, new_end);

        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        draw(&visible, log_scale, &ma_configs)?;
        LAST_RENDERED_CANDLES.with(|state| {
            *state.borrow_mut() = visible.clone();
        });
        render_status(&visible, log_scale, &ma_configs, None, None);
        Ok(())
    }

    // Reset the zoom/pan view so every loaded candle fits on screen, on both
    // axes: the x-axis (time) window spans all candles and the y-axis (price)
    // vertical stretch/pan is reset to its autoscaled default.
    fn auto_fit_view() -> Result<(), JsValue> {
        let candles = LAST_CANDLES.with(|state| state.borrow().clone());
        let Some((start, end)) = candle_bounds(&candles) else {
            set_status("Load candles before auto fit");
            return Ok(());
        };
        Y_STRETCH_FACTOR.with(|state| *state.borrow_mut() = 1.0);
        Y_PAN_LINEAR_OFFSET.with(|state| *state.borrow_mut() = 0.0);
        Y_PAN_LOG_OFFSET.with(|state| *state.borrow_mut() = 0.0);
        apply_range_change_client_only(start, (start + 60).max(end))?;
        set_status("View fit to all candles");
        Ok(())
    }

    // Keep the canvas backing buffer (its `width`/`height` attributes, which is
    // what plotters draws into) the same size CSS renders it at. Plotters measures
    // its fixed pixel margins (16/36/72) against the buffer, while the mouse->time
    // conversion measures those same margins against `client_width()`. If the two
    // diverge -- the buffer is a fixed size and CSS stretches it -- a drawn line or
    // price range lands offset from the cursor (shifted left when the rendered
    // width is narrower than the buffer). Forcing buffer == client size keeps both
    // coordinate spaces identical so figures land exactly where clicked.
    fn sync_canvas_backing_size(canvas: &HtmlCanvasElement) {
        let client_w = canvas.client_width();
        let client_h = canvas.client_height();
        if client_w > 0 && canvas.width() != client_w as u32 {
            canvas.set_width(client_w as u32);
        }
        if client_h > 0 && canvas.height() != client_h as u32 {
            canvas.set_height(client_h as u32);
        }
    }

    fn plot_bounds(canvas_width: f64, canvas_height: f64) -> Option<(f64, f64, f64, f64)> {
        if canvas_width <= 0.0 || canvas_height <= 0.0 {
            return None;
        }

        let margin = 16.0;
        let y_label_area = 72.0;
        let x_label_area = 36.0;

        let plot_left = margin + y_label_area;
        let plot_right = canvas_width - margin;
        let plot_top = margin;
        let plot_bottom = canvas_height - margin - x_label_area;

        if plot_right <= plot_left || plot_bottom <= plot_top {
            return None;
        }

        Some((plot_left, plot_right, plot_top, plot_bottom))
    }

    fn timestamp_from_canvas_x(
        canvas_width: f64,
        canvas_height: f64,
        offset_x: f64,
    ) -> Option<i64> {
        if canvas_width <= 0.0 {
            return None;
        }

        let (plot_left, plot_right, _, _) = plot_bounds(canvas_width, canvas_height)?;
        if plot_right <= plot_left {
            return None;
        }

        let clamped_x = offset_x.clamp(plot_left, plot_right);
        let ratio = ((clamped_x - plot_left) / (plot_right - plot_left)).clamp(0.0, 1.0);
        CHART_VIEW.with(|view| {
            let cfg = (*view.borrow())?;
            let span = (cfg.x_end - cfg.x_start).max(60) as f64;
            Some((cfg.x_start as f64 + ratio * span).round() as i64)
        })
    }

    fn price_from_canvas_y(offset_y: f64, plot_top: f64, plot_bottom: f64) -> Option<f64> {
        if offset_y < plot_top || offset_y > plot_bottom || plot_bottom <= plot_top {
            return None;
        }

        let raw_ratio = ((plot_bottom - offset_y) / (plot_bottom - plot_top)).clamp(0.0, 1.0);
        CHART_VIEW.with(|view| {
            let v = *view.borrow();
            v.and_then(|cfg| {
                let ratio = if cfg.flipped { 1.0 - raw_ratio } else { raw_ratio };
                if cfg.use_log_scale {
                    if cfg.y_low <= 0.0 || cfg.y_high <= 0.0 {
                        return None;
                    }
                    let low_ln = cfg.y_low.ln();
                    let high_ln = cfg.y_high.ln();
                    Some((low_ln + ratio * (high_ln - low_ln)).exp())
                } else {
                    Some(cfg.y_low + ratio * (cfg.y_high - cfg.y_low))
                }
            })
        })
    }

    fn set_y_stretch_factor(next: f64) -> bool {
        let clamped = next.clamp(0.2, 25.0);
        Y_STRETCH_FACTOR.with(|state| {
            let mut factor = state.borrow_mut();
            if (*factor - clamped).abs() < 0.001 {
                false
            } else {
                *factor = clamped;
                true
            }
        })
    }

    fn set_y_pan_linear_offset(next: f64) -> bool {
        Y_PAN_LINEAR_OFFSET.with(|state| {
            let mut offset = state.borrow_mut();
            if (*offset - next).abs() < 0.001 {
                false
            } else {
                *offset = next;
                true
            }
        })
    }

    fn set_y_pan_log_offset(next: f64) -> bool {
        Y_PAN_LOG_OFFSET.with(|state| {
            let mut offset = state.borrow_mut();
            if (*offset - next).abs() < 0.000_1 {
                false
            } else {
                *offset = next;
                true
            }
        })
    }

    fn apply_panned_range_delta(
        ts_start: i64,
        ts_end: i64,
        delta_seconds: f64,
        remainder: &'static std::thread::LocalKey<RefCell<f64>>,
    ) -> Result<bool, JsValue> {
        let whole_seconds = remainder.with(|state| {
            let mut carry = state.borrow_mut();
            let total = *carry + delta_seconds;
            let whole_seconds = total.trunc() as i64;
            *carry = total - whole_seconds as f64;
            whole_seconds
        });

        if whole_seconds == 0 {
            return Ok(false);
        }

        apply_range_change_client_only(ts_start + whole_seconds, ts_end + whole_seconds)?;
        Ok(true)
    }

    fn inferred_candle_spacing(candles: &[Candle]) -> i64 {
        candles
            .windows(2)
            .map(|pair| (pair[1].timestamp - pair[0].timestamp).abs())
            .find(|diff| *diff > 0)
            .unwrap_or(60)
            .max(60)
    }

    fn nearest_candle_for_timestamp(candles: &[Candle], ts: i64) -> Option<&Candle> {
        let spacing = inferred_candle_spacing(candles);
        candles
            .iter()
            .min_by_key(|candle| (candle.timestamp - ts).abs())
            .and_then(|candle| {
                if (candle.timestamp - ts).abs() <= spacing / 2 {
                    Some(candle)
                } else {
                    None
                }
            })
    }

    fn draw(
        candles: &[Candle],
        log_scale: bool,
        ma_configs: &[MovingAverageConfig],
    ) -> Result<(), JsValue> {
        let doc = document()?;
        let canvas = doc
            .get_element_by_id("chart")
            .ok_or_else(|| JsValue::from_str("missing chart canvas"))?
            .dyn_into::<HtmlCanvasElement>()?;

        sync_canvas_backing_size(&canvas);
        let backend = CanvasBackend::with_canvas_object(canvas)
            .ok_or_else(|| JsValue::from_str("canvas backend error"))?;
        let root = backend.into_drawing_area();
        root.fill(&RGBColor(246, 247, 251))
            .map_err(|e| JsValue::from_str(&format!("background error: {e}")))?;
        let (x_start, x_end) = rendered_range()
            .or_else(|| {
                candles.first().zip(candles.last()).map(|(first, last)| {
                    let start = first.timestamp.min(last.timestamp);
                    let end = first.timestamp.max(last.timestamp);
                    (start, (start + 60).max(end))
                })
            })
            .unwrap_or((0, 60));

        let price_bounds = if candles.is_empty() {
            LAST_CANDLES.with(|state| {
                let loaded = state.borrow();
                if loaded.is_empty() {
                    None
                } else {
                    let low = loaded
                        .iter()
                        .map(|c| c.low)
                        .fold(f64::INFINITY, |acc, v| acc.min(v));
                    let high = loaded
                        .iter()
                        .map(|c| c.high)
                        .fold(f64::NEG_INFINITY, |acc, v| acc.max(v));
                    Some((low, high))
                }
            })
        } else {
            Some((
                candles
                    .iter()
                    .map(|c| c.low)
                    .fold(f64::INFINITY, |acc, v| acc.min(v)),
                candles
                    .iter()
                    .map(|c| c.high)
                    .fold(f64::NEG_INFINITY, |acc, v| acc.max(v)),
            ))
        };
        let Some((raw_y_min, raw_y_max)) = price_bounds else {
            CHART_VIEW.with(|view| {
                *view.borrow_mut() = None;
            });
            root.draw(&Text::new(
                "No candles loaded",
                (24, 32),
                ("sans-serif", 22).into_font().color(&BLACK),
            ))
            .map_err(|e| JsValue::from_str(&format!("draw text error: {e}")))?;
            root.present()
                .map_err(|e| JsValue::from_str(&format!("present error: {e}")))?;
            update_last_price_tag(None);
            draw_rsi(candles)?;
            return Ok(());
        };
        let measure_range = active_measure_range();
        let measure_price_range = active_measure_price_range();

        // Keep autoscaling tied to market data so Fib extensions don't flatten the chart.
        let y_min_linear = raw_y_min;
        let y_max_linear = raw_y_max;
        let y_min_log = raw_y_min;
        let y_max_log = raw_y_max;

        let y_span_linear = (y_max_linear - y_min_linear).abs();
        let y_pad_linear = y_span_linear * 0.06;

        let use_log_scale = log_scale && y_min_log > 0.0;
        let stretch_factor = Y_STRETCH_FACTOR
            .with(|state| *state.borrow())
            .clamp(0.2, 25.0);
        let (y_low, y_high) = if use_log_scale {
            let low_ln = y_min_log.ln();
            let high_ln = y_max_log.ln();
            let center_ln = (low_ln + high_ln) / 2.0;
            // padding and stretch applied in log space — scale-independent
            let half_span_ln = ((high_ln - low_ln).abs() / 2.0).max(0.02) * 1.06 * stretch_factor;
            let pan_offset_ln = Y_PAN_LOG_OFFSET.with(|state| *state.borrow());
            let low = center_ln - half_span_ln + pan_offset_ln;
            let high = center_ln + half_span_ln + pan_offset_ln;
            (low.exp(), high.exp())
        } else {
            let base_low = y_min_linear - y_pad_linear;
            let base_high = y_max_linear + y_pad_linear;
            let center = (base_low + base_high) / 2.0;
            let half_span = ((base_high - base_low) / 2.0).max(1.0) * stretch_factor;
            let pan_offset = Y_PAN_LINEAR_OFFSET.with(|state| *state.borrow());
            (
                center - half_span + pan_offset,
                center + half_span + pan_offset,
            )
        };
        let chart_flipped = CHART_FLIPPED.with(|state| *state.borrow());
        CHART_VIEW.with(|view| {
            *view.borrow_mut() = Some(ChartView {
                x_start,
                x_end,
                y_low,
                y_high,
                use_log_scale,
                flipped: chart_flipped,
            });
        });
        // Compute moving averages over the full loaded dataset rather than the
        // visible slice, then clip to the view. This keeps each MA line stable
        // and continuous when zooming/panning instead of recomputing from
        // scratch (and losing its warmup) at the left edge of the window.
        let ma_full = LAST_CANDLES.with(|state| state.borrow().clone());
        let ma_source: &[Candle] = if ma_full.len() >= candles.len() {
            &ma_full
        } else {
            candles
        };
        let ma_series: Vec<(RGBColor, Vec<(i64, f64)>)> = ma_configs
            .iter()
            .filter(|cfg| cfg.enabled)
            .map(|cfg| {
                (
                    cfg.color,
                    clip_points_to_range(sma_points(ma_source, cfg.period), x_start, x_end),
                )
            })
            .filter(|(_, points)| !points.is_empty())
            .collect();
        let fib_renders = fib_renders(x_start, x_end, y_low, y_high, use_log_scale);
        let (measure_x_start, measure_x_end, measure_label_x) = measure_range
            .map(|(start, end)| {
                let start = start.clamp(x_start, x_end);
                let end = end.clamp(x_start, x_end);
                let label_x = start + ((end - start).max(60) / 2);
                (
                    start.min(end),
                    start.max(end),
                    label_x.clamp(x_start, x_end),
                )
            })
            .unwrap_or((x_start, x_start, x_start));
        let measure_label = measure_range.map(|(start, end)| format_duration_human(end - start));
        let measure_price_label =
            measure_price_range.map(|(start, end)| format_measure_price_label(start, end));
        let trend_lines = active_trend_lines();

        if use_log_scale {
            let mut chart = ChartBuilder::on(&root)
                .margin(16)
                .x_label_area_size(36)
                .y_label_area_size(72)
                .build_cartesian_2d(
                    x_start..x_end,
                    if chart_flipped {
                        (y_high..y_low).log_scale()
                    } else {
                        (y_low..y_high).log_scale()
                    },
                )
                .map_err(|e| JsValue::from_str(&format!("chart build error: {e}")))?;

            chart
                .configure_mesh()
                .x_labels(12)
                .disable_x_mesh()
                .disable_y_mesh()
                .x_label_formatter(&|x| unix_seconds_to_date_text(*x))
                .y_label_formatter(&|_| String::new())
                .draw()
                .map_err(|e| JsValue::from_str(&format!("mesh draw error: {e}")))?;

            for ty in y_tick_values(y_low, y_high, true, 10) {
                chart
                    .draw_series(LineSeries::new(
                        vec![(x_start, ty), (x_end, ty)],
                        RGBColor(224, 230, 240).mix(0.7),
                    ))
                    .map_err(|e| JsValue::from_str(&format!("y grid draw error: {e}")))?;
                chart
                    .draw_series(std::iter::once(Text::new(
                        format_price_label(ty),
                        (x_start, ty),
                        ("sans-serif", 11)
                            .into_font()
                            .color(&RGBColor(60, 80, 110))
                            .pos(Pos::new(HPos::Right, VPos::Center)),
                    )))
                    .map_err(|e| JsValue::from_str(&format!("y label draw error: {e}")))?;
            }

            chart
                .draw_series(candles.iter().map(|c| {
                    CandleStick::new(
                        c.timestamp,
                        c.open,
                        c.high,
                        c.low,
                        c.close,
                        RGBColor(29, 142, 76).filled(),
                        RGBColor(198, 56, 56).filled(),
                        6,
                    )
                }))
                .map_err(|e| JsValue::from_str(&format!("candles draw error: {e}")))?;

            for (color, points) in &ma_series {
                chart
                    .draw_series(LineSeries::new(points.clone(), color))
                    .map_err(|e| JsValue::from_str(&format!("ma draw error: {e}")))?;
            }

            if let Some(last) = candles.last() {
                if last.close.is_finite() && last.close >= y_low && last.close <= y_high {
                    let color = RGBColor(47, 125, 216);
                    chart
                        .draw_series(LineSeries::new(
                            vec![(x_start, last.close), (x_end, last.close)],
                            color.mix(0.9),
                        ))
                        .map_err(|e| JsValue::from_str(&format!("last line draw error: {e}")))?;
                }
            }

            for render in &fib_renders {
                if render.x_end > render.x_start {
                    chart
                        .draw_series([
                            PathElement::new(
                                vec![(render.x_start, y_low), (render.x_start, y_high)],
                                RGBColor(173, 104, 32).mix(0.25),
                            ),
                            PathElement::new(
                                vec![(render.x_end, y_low), (render.x_end, y_high)],
                                RGBColor(173, 104, 32).mix(0.25),
                            ),
                        ])
                        .map_err(|e| {
                            JsValue::from_str(&format!("fib boundary draw error: {e}"))
                        })?;
                }

                for (ratio, level_price) in &render.levels {
                    chart
                        .draw_series(LineSeries::new(
                            vec![(render.x_start, *level_price), (render.x_end, *level_price)],
                            &RGBColor(173, 104, 32),
                        ))
                        .map_err(|e| JsValue::from_str(&format!("fib draw error: {e}")))?;
                    chart
                        .draw_series(std::iter::once(Text::new(
                            format!(
                                "{} ({:.1}%)  {:.2}",
                                fib_ratio_label(*ratio),
                                ratio * 100.0,
                                level_price
                            ),
                            (render.label_x, *level_price),
                            ("sans-serif", 11).into_font().color(&RGBColor(122, 72, 24)),
                        )))
                        .map_err(|e| JsValue::from_str(&format!("fib label draw error: {e}")))?;
                }
            }

            for (a, b) in &trend_lines {
                chart
                    .draw_series(LineSeries::new(
                        vec![(a.0, a.1), (b.0, b.1)],
                        RGBColor(232, 122, 18).stroke_width(2),
                    ))
                    .map_err(|e| JsValue::from_str(&format!("trend line draw error: {e}")))?;
            }

            if let Some(label) = &measure_label {
                let measure_y = if y_low > 0.0 && y_high > y_low {
                    (y_low.ln() + (y_high.ln() - y_low.ln()) * 0.92).exp()
                } else {
                    y_high
                };
                chart
                    .draw_series([
                        PathElement::new(
                            vec![(measure_x_start, y_low), (measure_x_start, y_high)],
                            RGBColor(42, 54, 80).mix(0.35),
                        ),
                        PathElement::new(
                            vec![(measure_x_end, y_low), (measure_x_end, y_high)],
                            RGBColor(42, 54, 80).mix(0.35),
                        ),
                    ])
                    .map_err(|e| JsValue::from_str(&format!("measure boundary draw error: {e}")))?;
                chart
                    .draw_series(LineSeries::new(
                        vec![(measure_x_start, measure_y), (measure_x_end, measure_y)],
                        &RGBColor(42, 54, 80),
                    ))
                    .map_err(|e| JsValue::from_str(&format!("measure draw error: {e}")))?;
                chart
                    .draw_series(std::iter::once(Text::new(
                        label.clone(),
                        (measure_label_x, measure_y),
                        ("sans-serif", 12).into_font().color(&RGBColor(42, 54, 80)),
                    )))
                    .map_err(|e| JsValue::from_str(&format!("measure label draw error: {e}")))?;
            }

            if let (Some((start_price, end_price)), Some(label)) =
                (measure_price_range, &measure_price_label)
            {
                let low_price = start_price.min(end_price);
                let high_price = start_price.max(end_price);
                let label_y = if low_price > 0.0 && high_price > low_price {
                    (low_price.ln() + (high_price.ln() - low_price.ln()) / 2.0).exp()
                } else {
                    (start_price + end_price) / 2.0
                }
                .clamp(y_low, y_high);
                chart
                    .draw_series([
                        PathElement::new(
                            vec![(measure_x_start, start_price), (measure_x_end, start_price)],
                            RGBColor(42, 54, 80).mix(0.25),
                        ),
                        PathElement::new(
                            vec![(measure_x_start, end_price), (measure_x_end, end_price)],
                            RGBColor(42, 54, 80).mix(0.25),
                        ),
                        PathElement::new(
                            vec![(measure_x_end, start_price), (measure_x_end, end_price)],
                            RGBColor(42, 54, 80),
                        ),
                    ])
                    .map_err(|e| JsValue::from_str(&format!("measure price draw error: {e}")))?;
                chart
                    .draw_series(std::iter::once(Text::new(
                        label.clone(),
                        (measure_x_end, label_y),
                        ("sans-serif", 12)
                            .into_font()
                            .color(&RGBColor(42, 54, 80))
                            .pos(Pos::new(HPos::Left, VPos::Center)),
                    )))
                    .map_err(|e| {
                        JsValue::from_str(&format!("measure price label draw error: {e}"))
                    })?;
            }
        } else {
            let mut chart = ChartBuilder::on(&root)
                .margin(16)
                .x_label_area_size(36)
                .y_label_area_size(72)
                .build_cartesian_2d(
                    x_start..x_end,
                    if chart_flipped { y_high..y_low } else { y_low..y_high },
                )
                .map_err(|e| JsValue::from_str(&format!("chart build error: {e}")))?;

            chart
                .configure_mesh()
                .x_labels(12)
                .disable_x_mesh()
                .disable_y_mesh()
                .x_label_formatter(&|x| unix_seconds_to_date_text(*x))
                .y_label_formatter(&|_| String::new())
                .draw()
                .map_err(|e| JsValue::from_str(&format!("mesh draw error: {e}")))?;

            for ty in y_tick_values(y_low, y_high, false, 10) {
                chart
                    .draw_series(LineSeries::new(
                        vec![(x_start, ty), (x_end, ty)],
                        RGBColor(224, 230, 240).mix(0.7),
                    ))
                    .map_err(|e| JsValue::from_str(&format!("y grid draw error: {e}")))?;
                chart
                    .draw_series(std::iter::once(Text::new(
                        format_price_label(ty),
                        (x_start, ty),
                        ("sans-serif", 11)
                            .into_font()
                            .color(&RGBColor(60, 80, 110))
                            .pos(Pos::new(HPos::Right, VPos::Center)),
                    )))
                    .map_err(|e| JsValue::from_str(&format!("y label draw error: {e}")))?;
            }

            chart
                .draw_series(candles.iter().map(|c| {
                    CandleStick::new(
                        c.timestamp,
                        c.open,
                        c.high,
                        c.low,
                        c.close,
                        RGBColor(29, 142, 76).filled(),
                        RGBColor(198, 56, 56).filled(),
                        6,
                    )
                }))
                .map_err(|e| JsValue::from_str(&format!("candles draw error: {e}")))?;

            for (color, points) in &ma_series {
                chart
                    .draw_series(LineSeries::new(points.clone(), color))
                    .map_err(|e| JsValue::from_str(&format!("ma draw error: {e}")))?;
            }

            if let Some(last) = candles.last() {
                if last.close.is_finite() && last.close >= y_low && last.close <= y_high {
                    let color = RGBColor(47, 125, 216);
                    chart
                        .draw_series(LineSeries::new(
                            vec![(x_start, last.close), (x_end, last.close)],
                            color.mix(0.9),
                        ))
                        .map_err(|e| JsValue::from_str(&format!("last line draw error: {e}")))?;
                }
            }

            for render in &fib_renders {
                if render.x_end > render.x_start {
                    chart
                        .draw_series([
                            PathElement::new(
                                vec![(render.x_start, y_low), (render.x_start, y_high)],
                                RGBColor(173, 104, 32).mix(0.25),
                            ),
                            PathElement::new(
                                vec![(render.x_end, y_low), (render.x_end, y_high)],
                                RGBColor(173, 104, 32).mix(0.25),
                            ),
                        ])
                        .map_err(|e| {
                            JsValue::from_str(&format!("fib boundary draw error: {e}"))
                        })?;
                }

                for (ratio, level_price) in &render.levels {
                    chart
                        .draw_series(LineSeries::new(
                            vec![(render.x_start, *level_price), (render.x_end, *level_price)],
                            &RGBColor(173, 104, 32),
                        ))
                        .map_err(|e| JsValue::from_str(&format!("fib draw error: {e}")))?;
                    chart
                        .draw_series(std::iter::once(Text::new(
                            format!(
                                "{} ({:.1}%)  {:.2}",
                                fib_ratio_label(*ratio),
                                ratio * 100.0,
                                level_price
                            ),
                            (render.label_x, *level_price),
                            ("sans-serif", 11).into_font().color(&RGBColor(122, 72, 24)),
                        )))
                        .map_err(|e| JsValue::from_str(&format!("fib label draw error: {e}")))?;
                }
            }

            if let Some(label) = &measure_label {
                let measure_y = y_low + (y_high - y_low) * 0.92;
                chart
                    .draw_series([
                        PathElement::new(
                            vec![(measure_x_start, y_low), (measure_x_start, y_high)],
                            RGBColor(42, 54, 80).mix(0.35),
                        ),
                        PathElement::new(
                            vec![(measure_x_end, y_low), (measure_x_end, y_high)],
                            RGBColor(42, 54, 80).mix(0.35),
                        ),
                    ])
                    .map_err(|e| JsValue::from_str(&format!("measure boundary draw error: {e}")))?;
                chart
                    .draw_series(LineSeries::new(
                        vec![(measure_x_start, measure_y), (measure_x_end, measure_y)],
                        &RGBColor(42, 54, 80),
                    ))
                    .map_err(|e| JsValue::from_str(&format!("measure draw error: {e}")))?;
                chart
                    .draw_series(std::iter::once(Text::new(
                        label.clone(),
                        (measure_label_x, measure_y),
                        ("sans-serif", 12).into_font().color(&RGBColor(42, 54, 80)),
                    )))
                    .map_err(|e| JsValue::from_str(&format!("measure label draw error: {e}")))?;
            }

            if let (Some((start_price, end_price)), Some(label)) =
                (measure_price_range, &measure_price_label)
            {
                let label_y = ((start_price + end_price) / 2.0).clamp(y_low, y_high);
                chart
                    .draw_series([
                        PathElement::new(
                            vec![(measure_x_start, start_price), (measure_x_end, start_price)],
                            RGBColor(42, 54, 80).mix(0.25),
                        ),
                        PathElement::new(
                            vec![(measure_x_start, end_price), (measure_x_end, end_price)],
                            RGBColor(42, 54, 80).mix(0.25),
                        ),
                        PathElement::new(
                            vec![(measure_x_end, start_price), (measure_x_end, end_price)],
                            RGBColor(42, 54, 80),
                        ),
                    ])
                    .map_err(|e| JsValue::from_str(&format!("measure price draw error: {e}")))?;
                chart
                    .draw_series(std::iter::once(Text::new(
                        label.clone(),
                        (measure_x_end, label_y),
                        ("sans-serif", 12)
                            .into_font()
                            .color(&RGBColor(42, 54, 80))
                            .pos(Pos::new(HPos::Left, VPos::Center)),
                    )))
                    .map_err(|e| {
                        JsValue::from_str(&format!("measure price label draw error: {e}"))
                    })?;
            }
        }

        root.present()
            .map_err(|e| JsValue::from_str(&format!("present error: {e}")))?;

        update_last_price_tag(candles.last().map(|c| c.close));

        draw_rsi(candles)?;

        Ok(())
    }

    fn render_status(
        candles: &[Candle],
        log_scale: bool,
        ma_configs: &[MovingAverageConfig],
        request_ms: Option<f64>,
        total_ms: Option<f64>,
    ) {
        let first_ts = candles.first().map(|c| c.timestamp).unwrap_or_default();
        let last_ts = candles.last().map(|c| c.timestamp).unwrap_or_default();
        let first_text = unix_seconds_to_hover_text(first_ts);
        let last_text = unix_seconds_to_hover_text(last_ts);
        let total_volume: f64 = candles.iter().map(|c| c.volume).sum();
        let period_label = input_value("period").unwrap_or_else(|_| "unknown".to_string());
        let scale_label = if log_scale { "log" } else { "linear" };
        let active: Vec<String> = ma_configs
            .iter()
            .filter(|cfg| cfg.enabled)
            .map(|cfg| format!("MA{}({})", cfg.idx, cfg.period))
            .collect();
        let ma_label = if active.is_empty() {
            "MA off".to_string()
        } else {
            active.join(",")
        };

        if let (Some(request_ms), Some(total_ms)) = (request_ms, total_ms) {
            set_status(&format!(
                "Loaded {} candles from {} to {} | period {} | total volume {:.4} | scale {} | {} | request {:.0}ms | total {:.0}ms",
                candles.len(),
                first_text,
                last_text,
                period_label,
                total_volume,
                scale_label,
                ma_label,
                request_ms,
                total_ms
            ));
        } else {
            set_status(&format!(
                "Loaded {} candles from {} to {} | period {} | total volume {:.4} | scale {} | {} | rerender client-side",
                candles.len(),
                first_text,
                last_text,
                period_label,
                total_volume,
                scale_label,
                ma_label
            ));
        }
    }

    async fn rerender_cached_or_fetch() -> Result<(), JsValue> {
        save_inputs()?;
        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        let candles = LAST_CANDLES.with(|state| state.borrow().clone());
        if candles.is_empty() {
            return fetch_and_draw().await;
        }

        let (ts_start, ts_end) = match rendered_range() {
            Some(v) => v,
            None => selected_ts_range()?,
        };
        let (ts_start, ts_end) = clamp_range_to_loaded(ts_start, ts_end);
        CLIENT_VIEW_RANGE.with(|state| {
            *state.borrow_mut() = Some((ts_start, ts_end));
        });
        save_view_range(ts_start, ts_end);
        let visible = filter_candles_by_range(&candles, ts_start, ts_end);

        draw(&visible, log_scale, &ma_configs)?;
        LAST_RENDERED_CANDLES.with(|state| {
            *state.borrow_mut() = visible.clone();
        });
        render_status(&visible, log_scale, &ma_configs, None, None);
        Ok(())
    }

    async fn fetch_candles(url: &str) -> Result<Vec<Candle>, JsValue> {
        let resp = Request::get(url)
            .send()
            .await
            .map_err(|e| JsValue::from_str(&format!("request failed: {e}")))?;
        if !resp.ok() {
            let body = resp.text().await.unwrap_or_default();
            return Err(JsValue::from_str(&format!(
                "API error {}: {}",
                resp.status(),
                body
            )));
        }
        resp.json::<Vec<Candle>>()
            .await
            .map_err(|e| JsValue::from_str(&format!("invalid JSON response: {e}")))
    }

    async fn fetch_and_draw() -> Result<(), JsValue> {
        let started_at = Date::now();
        save_inputs()?;
        let request = build_request()?;
        let log_scale = checkbox_checked("log-scale")?;
        let ma_configs = moving_average_configs()?;
        set_status("Loading candles...");

        let request_started_at = Date::now();
        let candles = match request {
            ChartRequest::Direct(url) => match fetch_candles(&url).await {
                Ok(candles) => candles,
                Err(err) => {
                    set_status(&err.as_string().unwrap_or_else(|| "request failed".into()));
                    return Ok(());
                }
            },
            ChartRequest::Ratio { num_url, den_url } => {
                let num = match fetch_candles(&num_url).await {
                    Ok(c) => c,
                    Err(err) => {
                        set_status(&err.as_string().unwrap_or_else(|| "request failed".into()));
                        return Ok(());
                    }
                };
                let den = match fetch_candles(&den_url).await {
                    Ok(c) => c,
                    Err(err) => {
                        set_status(&err.as_string().unwrap_or_else(|| "request failed".into()));
                        return Ok(());
                    }
                };
                compute_ratio_candles(&num, &den)
            }
        };
        let request_ms = Date::now() - request_started_at;

        LAST_CANDLES.with(|state| {
            *state.borrow_mut() = candles.clone();
        });
        let full_bounds = candle_bounds(&candles)
            .map(|(start, end)| (start, (start + 60).max(end)))
            .unwrap_or_else(|| {
                let now_secs = (Date::now() / 1000.0) as i64;
                (now_secs.saturating_sub(60), now_secs)
            });
        // Keep the previous zoom/pan view across reloads when the timeframe
        // (period) is unchanged; otherwise show the full loaded range.
        let (view_start, view_end) = match saved_view_range_for_current_period() {
            Some((s, e)) => clamp_range_to_loaded(s, e),
            None => full_bounds,
        };
        CLIENT_VIEW_RANGE.with(|state| {
            *state.borrow_mut() = Some((view_start, view_end));
        });
        save_view_range(view_start, view_end);
        let visible = filter_candles_by_range(&candles, view_start, view_end);
        draw(&visible, log_scale, &ma_configs)?;
        let total_ms = Date::now() - started_at;
        LAST_RENDERED_CANDLES.with(|state| {
            *state.borrow_mut() = visible.clone();
        });
        render_status(
            &visible,
            log_scale,
            &ma_configs,
            Some(request_ms),
            Some(total_ms),
        );

        Ok(())
    }

    fn setup_defaults() -> Result<(), JsValue> {
        load_saved_inputs()?;
        // Restore the saved drawings for whichever pair is selected on load.
        let pair = current_pair_key();
        load_pair_drawings(&pair);
        CURRENT_PAIR_KEY.with(|cur| {
            *cur.borrow_mut() = pair;
        });
        // Reusable timeout callback that hides the on-chart trash icon.
        FIGURE_TRASH_HIDE_CLOSURE.with(|closure| {
            if closure.borrow().is_none() {
                *closure.borrow_mut() =
                    Some(Closure::wrap(Box::new(hide_figure_trash) as Box<dyn FnMut()>));
            }
        });
        sync_fib_button()?;
        sync_stretch_button()?;
        sync_auto_mode_button()?;
        sync_measure_button()?;
        set_fib_popup_info("Move cursor over chart to use Fibonacci tool");

        let now_secs = (Date::now() / 1000.0) as i64;
        let back_30_days = now_secs - 30 * 24 * 60 * 60;
        if input_value("ts-start-human")?.is_empty() {
            set_input_value(
                "ts-start-human",
                &unix_seconds_to_datetime_local(back_30_days),
            )?;
        }
        if input_value("ts-end-human")?.is_empty() {
            set_input_value("ts-end-human", &unix_seconds_to_datetime_local(now_secs))?;
        }

        save_inputs()?;
        Ok(())
    }

    fn register_button_handler() -> Result<(), JsValue> {
        let doc = document()?;
        let load_button = doc
            .get_element_by_id("load")
            .ok_or_else(|| JsValue::from_str("missing load button"))?;
        let api_base_input = doc
            .get_element_by_id("api-base")
            .ok_or_else(|| JsValue::from_str("missing api-base input"))?;
        let db_select = doc
            .get_element_by_id("db")
            .ok_or_else(|| JsValue::from_str("missing db select"))?;
        let chart_source_select = doc
            .get_element_by_id("chart-source")
            .ok_or_else(|| JsValue::from_str("missing chart-source select"))?;
        let log_scale_toggle_button = doc
            .get_element_by_id("log-scale-toggle")
            .ok_or_else(|| JsValue::from_str("missing log scale toggle button"))?;
        let stretch_toggle_button = doc
            .get_element_by_id("stretch-toggle")
            .ok_or_else(|| JsValue::from_str("missing stretch toggle button"))?;
        let measure_toggle_button = doc
            .get_element_by_id("measure-toggle")
            .ok_or_else(|| JsValue::from_str("missing measure toggle button"))?;
        let fib_toggle_button = doc
            .get_element_by_id("fib-toggle")
            .ok_or_else(|| JsValue::from_str("missing fib toggle button"))?;
        let line_toggle_button = doc
            .get_element_by_id("line-toggle")
            .ok_or_else(|| JsValue::from_str("missing line toggle button"))?;
        let figure_trash_button = doc
            .get_element_by_id("figure-trash")
            .ok_or_else(|| JsValue::from_str("missing figure trash button"))?;
        let auto_fit_button = doc
            .get_element_by_id("auto-fit")
            .ok_or_else(|| JsValue::from_str("missing auto fit button"))?;
        let auto_mode_toggle_button = doc
            .get_element_by_id("auto-mode-toggle")
            .ok_or_else(|| JsValue::from_str("missing auto mode toggle button"))?;
        let flip_chart_button = doc
            .get_element_by_id("flip-chart")
            .ok_or_else(|| JsValue::from_str("missing flip chart button"))?;
        let fib_list_container = doc
            .get_element_by_id("fib-list")
            .ok_or_else(|| JsValue::from_str("missing fib list"))?;
        let lines_list_container = doc
            .get_element_by_id("lines-list")
            .ok_or_else(|| JsValue::from_str("missing lines list"))?;
        let fib_popup = doc
            .get_element_by_id("fib-popup")
            .ok_or_else(|| JsValue::from_str("missing fib popup"))?
            .dyn_into::<HtmlElement>()?;
        let fib_popup_drag_handle = doc
            .get_element_by_id("fib-popup-drag-handle")
            .ok_or_else(|| JsValue::from_str("missing fib popup drag handle"))?;
        let settings_toggle_button = doc
            .get_element_by_id("settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing settings toggle button"))?;
        let settings_side_toggle_button = doc.get_element_by_id("settings-side-toggle");
        let ma_settings_drag_handle = doc
            .get_element_by_id("ma-settings-drag-handle")
            .ok_or_else(|| JsValue::from_str("missing ma settings drag handle"))?;
        let ma_settings_card = doc
            .get_element_by_id("ma-settings-card")
            .ok_or_else(|| JsValue::from_str("missing ma settings card"))?
            .dyn_into::<HtmlElement>()?;
        let connection_settings_toggle_button = doc
            .get_element_by_id("connection-settings-toggle")
            .ok_or_else(|| JsValue::from_str("missing connection settings toggle button"))?;
        let connection_settings_side_toggle_button =
            doc.get_element_by_id("connection-settings-side-toggle");
        let connection_settings_drag_handle = doc
            .get_element_by_id("connection-settings-drag-handle")
            .ok_or_else(|| JsValue::from_str("missing connection settings drag handle"))?;
        let connection_settings_card = doc
            .get_element_by_id("connection-settings-card")
            .ok_or_else(|| JsValue::from_str("missing connection settings card"))?
            .dyn_into::<HtmlElement>()?;
        let chart_canvas = doc
            .get_element_by_id("chart")
            .ok_or_else(|| JsValue::from_str("missing chart canvas"))?
            .dyn_into::<HtmlCanvasElement>()?;

        let load_callback = Closure::wrap(Box::new(move || {
            if let Err(err) = set_load_button_loading(true) {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = fetch_and_draw().await {
                    set_status(&format!("failed: {:?}", err));
                }
                if let Err(err) = set_load_button_loading(false) {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        load_button
            .add_event_listener_with_callback("click", load_callback.as_ref().unchecked_ref())?;
        load_callback.forget();

        let api_base_change_callback = Closure::wrap(Box::new(move || {
            if let Err(err) = save_inputs() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = connect_realtime_ws() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = set_load_button_loading(true) {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = fetch_and_draw().await {
                    set_status(&format!("failed: {:?}", err));
                }
                if let Err(err) = set_load_button_loading(false) {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        api_base_input.add_event_listener_with_callback(
            "change",
            api_base_change_callback.as_ref().unchecked_ref(),
        )?;
        api_base_change_callback.forget();

        let db_change_callback = Closure::wrap(Box::new(move || {
            if let Err(err) = save_inputs() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = connect_realtime_ws() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = set_load_button_loading(true) {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = fetch_and_draw().await {
                    set_status(&format!("failed: {:?}", err));
                }
                if let Err(err) = set_load_button_loading(false) {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        db_select.add_event_listener_with_callback(
            "change",
            db_change_callback.as_ref().unchecked_ref(),
        )?;
        db_change_callback.forget();

        let chart_source_change_callback = Closure::wrap(Box::new(move || {
            switch_pair_drawings(&current_pair_key());
            if let Err(err) = save_inputs() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = connect_realtime_ws() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = set_load_button_loading(true) {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = fetch_and_draw().await {
                    set_status(&format!("failed: {:?}", err));
                }
                if let Err(err) = set_load_button_loading(false) {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        chart_source_select.add_event_listener_with_callback(
            "change",
            chart_source_change_callback.as_ref().unchecked_ref(),
        )?;
        chart_source_change_callback.forget();

        let keydown_callback = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if (event.ctrl_key() || event.meta_key()) && event.key().eq_ignore_ascii_case("z") {
                event.prevent_default();
                undo_last_range_change();
                return;
            }

            if event.key() == "Escape" {
                event.prevent_default();
                if let Err(err) = cancel_active_tools() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                set_status("Tools off (drawings kept — use the trash icons to clear)");
                set_fib_popup_info("Tools off. Drawings kept. Click Fib to start again.");
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);

        doc.add_event_listener_with_callback("keydown", keydown_callback.as_ref().unchecked_ref())?;
        keydown_callback.forget();

        let wheel_callback = Closure::wrap(Box::new(move |event: WheelEvent| {
            event.prevent_default();

            let (cur_start, cur_end) = match rendered_range() {
                Some(v) => v,
                None => return,
            };

            let use_pan = event.shift_key() || event.delta_x().abs() > event.delta_y().abs();
            if use_pan {
                let delta = if event.delta_x().abs() > 0.0 {
                    event.delta_x()
                } else {
                    event.delta_y()
                };
                let span = (cur_end - cur_start).max(60) as f64;
                let delta_seconds = (delta / 240.0) * (span * 0.10).max(60.0);
                if let Err(err) = apply_panned_range_delta(
                    cur_start,
                    cur_end,
                    delta_seconds,
                    &WHEEL_PAN_REMAINDER,
                ) {
                    set_status(&format!("failed to pan: {:?}", err));
                }
            } else {
                let delta = event.delta_y();
                let normalized_delta = delta.clamp(-240.0, 240.0);
                let factor = (normalized_delta / 2400.0).exp();
                let (new_start, new_end) = zoomed_range_from(cur_start, cur_end, factor);
                if let Err(err) = apply_range_change_client_only(new_start, new_end) {
                    set_status(&format!("failed to zoom: {:?}", err));
                }
            }
        }) as Box<dyn FnMut(WheelEvent)>);

        chart_canvas
            .add_event_listener_with_callback("wheel", wheel_callback.as_ref().unchecked_ref())?;
        wheel_callback.forget();

        let log_scale_callback = Closure::wrap(Box::new(move || {
            let enabled = checkbox_checked("log-scale").unwrap_or(false);
            if let Err(err) = set_checkbox_checked("log-scale", !enabled) {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = sync_log_scale_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        log_scale_toggle_button.add_event_listener_with_callback(
            "click",
            log_scale_callback.as_ref().unchecked_ref(),
        )?;
        log_scale_callback.forget();

        let stretch_toggle_callback = Closure::wrap(Box::new(move || {
            let next_enabled = STRETCH_TOOL_ENABLED.with(|state| {
                let mut enabled = state.borrow_mut();
                *enabled = !*enabled;
                *enabled
            });
            if next_enabled {
                FIB_STATE.with(|state| {
                    state.borrow_mut().enabled = false;
                });
                MEASURE_STATE.with(|state| {
                    let mut cfg = state.borrow_mut();
                    cfg.enabled = false;
                    cfg.anchor_a = None;
                    cfg.anchor_b = None;
                });
                MEASURE_DRAG_TS.with(|state| {
                    *state.borrow_mut() = None;
                });
                MEASURE_DRAG_PRICE.with(|state| {
                    *state.borrow_mut() = None;
                });
                let _ = set_fib_preview_point(None);
                if let Err(err) = sync_fib_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                if let Err(err) = sync_measure_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                if let Err(err) = disable_line_tool() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                set_status("Stretch mode: drag up/down to zoom Y axis");
                set_fib_popup_info("Stretch mode on. Drag up/down to zoom Y axis.");
            } else {
                set_status("Drag mode active (default)");
                set_fib_popup_info("Move cursor over chart to use Fibonacci tool");
            }
            if let Err(err) = sync_stretch_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
        }) as Box<dyn FnMut()>);

        stretch_toggle_button.add_event_listener_with_callback(
            "click",
            stretch_toggle_callback.as_ref().unchecked_ref(),
        )?;
        stretch_toggle_callback.forget();

        let measure_toggle_callback = Closure::wrap(Box::new(move || {
            let next_enabled = MEASURE_STATE.with(|state| {
                let mut cfg = state.borrow_mut();
                cfg.enabled = !cfg.enabled;
                if cfg.enabled {
                    cfg.anchor_a = None;
                    cfg.anchor_b = None;
                }
                cfg.enabled
            });
            MEASURE_DRAG_TS.with(|state| {
                *state.borrow_mut() = None;
            });
            MEASURE_DRAG_PRICE.with(|state| {
                *state.borrow_mut() = None;
            });
            if next_enabled {
                FIB_STATE.with(|state| {
                    state.borrow_mut().enabled = false;
                });
                STRETCH_TOOL_ENABLED.with(|state| {
                    *state.borrow_mut() = false;
                });
                let _ = set_fib_preview_point(None);
                if let Err(err) = sync_fib_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                if let Err(err) = sync_stretch_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                if let Err(err) = disable_line_tool() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                set_status("Price % tool enabled");
                set_hover_info("Price %: click A on chart, drag to B");
            } else {
                set_status("Price % tool disabled");
            }
            if let Err(err) = sync_measure_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if let Err(err) = redraw_visible_chart_only() {
                set_status(&format!("failed: {:?}", err));
            }
        }) as Box<dyn FnMut()>);

        measure_toggle_button.add_event_listener_with_callback(
            "click",
            measure_toggle_callback.as_ref().unchecked_ref(),
        )?;
        measure_toggle_callback.forget();

        let fib_toggle_callback = Closure::wrap(Box::new(move || {
            FIB_STATE.with(|state| {
                let mut cfg = state.borrow_mut();
                cfg.enabled = !cfg.enabled;
            });
            let fib_enabled = FIB_STATE.with(|state| state.borrow().enabled);
            if fib_enabled {
                STRETCH_TOOL_ENABLED.with(|state| {
                    *state.borrow_mut() = false;
                });
                MEASURE_STATE.with(|state| {
                    let mut cfg = state.borrow_mut();
                    cfg.enabled = false;
                    cfg.anchor_a = None;
                    cfg.anchor_b = None;
                });
                MEASURE_DRAG_TS.with(|state| {
                    *state.borrow_mut() = None;
                });
                MEASURE_DRAG_PRICE.with(|state| {
                    *state.borrow_mut() = None;
                });
                if let Err(err) = sync_stretch_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                if let Err(err) = sync_measure_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
            }
            if fib_enabled {
                if let Err(err) = disable_line_tool() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
            } else {
                let _ = set_fib_preview_point(None);
            }
            if let Err(err) = sync_fib_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if fib_enabled {
                let has_draft = FIB_STATE.with(|state| state.borrow().draft.is_some());
                if has_draft {
                    set_status("Fib tool: click second point");
                    set_fib_popup_info("Click the second point on the chart.");
                } else {
                    set_status("Fib tool: click two points to add a fib");
                    set_fib_popup_info("Fib is on. Click two points to add a fib.");
                }
            } else {
                set_status("Fib tool disabled");
                set_fib_popup_info("Fib is off. Existing fibs preserved.");
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        fib_toggle_button.add_event_listener_with_callback(
            "click",
            fib_toggle_callback.as_ref().unchecked_ref(),
        )?;
        fib_toggle_callback.forget();

        let line_toggle_callback = Closure::wrap(Box::new(move || {
            let next_enabled = LINE_TOOL_ENABLED.with(|state| {
                let next = !*state.borrow();
                *state.borrow_mut() = next;
                next
            });
            if next_enabled {
                LINE_DRAFT_ANCHOR.with(|state| {
                    *state.borrow_mut() = None;
                });
                LINE_PREVIEW_POINT.with(|state| {
                    *state.borrow_mut() = None;
                });
            }
            if next_enabled {
                if let Err(err) = disable_tools_for_line() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
            }
            if let Err(err) = sync_line_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if next_enabled {
                set_status("Line tool: click two points per line");
            } else {
                set_status("Line tool disabled");
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);

        line_toggle_button.add_event_listener_with_callback(
            "click",
            line_toggle_callback.as_ref().unchecked_ref(),
        )?;
        line_toggle_callback.forget();

        let figure_trash_click_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            event.stop_propagation();
            if let Some(target) = FIGURE_TRASH_TARGET.with(|state| *state.borrow()) {
                delete_figure(target);
                set_status("Drawing deleted");
            }
            hide_figure_trash();
            if let Err(err) = redraw_visible_chart_only() {
                set_status(&format!("failed: {:?}", err));
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        figure_trash_button.add_event_listener_with_callback(
            "click",
            figure_trash_click_callback.as_ref().unchecked_ref(),
        )?;
        figure_trash_click_callback.forget();

        // Keep the icon up while the cursor is on it; restart the grace timer when
        // the cursor leaves it.
        let figure_trash_enter_callback = Closure::wrap(Box::new(move || {
            cancel_figure_trash_timer();
        }) as Box<dyn FnMut()>);
        figure_trash_button.add_event_listener_with_callback(
            "mouseenter",
            figure_trash_enter_callback.as_ref().unchecked_ref(),
        )?;
        figure_trash_enter_callback.forget();

        let figure_trash_leave_callback = Closure::wrap(Box::new(move || {
            schedule_hide_figure_trash();
        }) as Box<dyn FnMut()>);
        figure_trash_button.add_event_listener_with_callback(
            "mouseleave",
            figure_trash_leave_callback.as_ref().unchecked_ref(),
        )?;
        figure_trash_leave_callback.forget();

        let auto_fit_callback = Closure::wrap(Box::new(move || {
            if let Err(err) = auto_fit_view() {
                set_status(&format!("failed: {:?}", err));
            }
        }) as Box<dyn FnMut()>);

        auto_fit_button.add_event_listener_with_callback(
            "click",
            auto_fit_callback.as_ref().unchecked_ref(),
        )?;
        auto_fit_callback.forget();

        let auto_mode_callback = Closure::wrap(Box::new(move || {
            let next_enabled = AUTO_MODE_ENABLED.with(|state| {
                let mut val = state.borrow_mut();
                *val = !*val;
                *val
            });
            if let Err(err) = sync_auto_mode_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            if next_enabled {
                if let Err(err) = redraw_visible_chart_only() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                set_status("Auto mode on: Y axis fits visible candles");
            } else {
                set_status("Auto mode off");
            }
        }) as Box<dyn FnMut()>);

        auto_mode_toggle_button.add_event_listener_with_callback(
            "click",
            auto_mode_callback.as_ref().unchecked_ref(),
        )?;
        auto_mode_callback.forget();

        let flip_callback = Closure::wrap(Box::new(move || {
            // Persist drawings under the pre-flip key before toggling.
            persist_current_pair_drawings();
            let next = CHART_FLIPPED.with(|state| {
                let mut val = state.borrow_mut();
                *val = !*val;
                *val
            });
            if let Ok(storage) = storage() {
                let _ = storage.set_item(STORAGE_KEY_CHART_FLIPPED, if next { "1" } else { "0" });
            }
            // Load drawings for the new flip-qualified key.
            let new_key = current_pair_key();
            load_pair_drawings(&new_key);
            CURRENT_PAIR_KEY.with(|cur| {
                *cur.borrow_mut() = new_key;
            });
            if let Err(err) = sync_flip_button() {
                set_status(&format!("failed: {:?}", err));
                return;
            }
            spawn_local(async {
                if let Err(err) = rerender_cached_or_fetch().await {
                    set_status(&format!("failed: {:?}", err));
                }
            });
        }) as Box<dyn FnMut()>);
        flip_chart_button.add_event_listener_with_callback(
            "click",
            flip_callback.as_ref().unchecked_ref(),
        )?;
        flip_callback.forget();

        let fib_list_click = Closure::wrap(Box::new(move |event: MouseEvent| {
            let target = match event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                Some(el) => el,
                None => return,
            };
            if let Some(idx_str) = target.get_attribute("data-fib-idx") {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    delete_figure(FigureTarget::Fib(idx));
                    spawn_local(async {
                        if let Err(err) = rerender_cached_or_fetch().await {
                            set_status(&format!("failed: {:?}", err));
                        }
                    });
                }
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        fib_list_container.add_event_listener_with_callback(
            "click",
            fib_list_click.as_ref().unchecked_ref(),
        )?;
        fib_list_click.forget();

        let lines_list_click = Closure::wrap(Box::new(move |event: MouseEvent| {
            let target = match event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                Some(el) => el,
                None => return,
            };
            if let Some(idx_str) = target.get_attribute("data-line-idx") {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    delete_figure(FigureTarget::TrendLine(idx));
                    spawn_local(async {
                        if let Err(err) = rerender_cached_or_fetch().await {
                            set_status(&format!("failed: {:?}", err));
                        }
                    });
                }
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        lines_list_container.add_event_listener_with_callback(
            "click",
            lines_list_click.as_ref().unchecked_ref(),
        )?;
        lines_list_click.forget();

        let fib_drag_popup = fib_popup.clone();
        let fib_drag_start_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            event.prevent_default();
            let rect = fib_drag_popup.get_bounding_client_rect();
            let offset_x = event.client_x() as f64 - rect.left();
            let offset_y = event.client_y() as f64 - rect.top();
            FIB_POPUP_DRAG.with(|state| {
                *state.borrow_mut() = Some((offset_x, offset_y));
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        fib_popup_drag_handle.add_event_listener_with_callback(
            "mousedown",
            fib_drag_start_callback.as_ref().unchecked_ref(),
        )?;
        fib_drag_start_callback.forget();

        let fib_drag_move_popup = fib_popup.clone();
        let fib_drag_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            FIB_POPUP_DRAG.with(|state| {
                if let Some((offset_x, offset_y)) = *state.borrow() {
                    let popup_rect = fib_drag_move_popup.get_bounding_client_rect();
                    let mut left = event.client_x() as f64 - offset_x;
                    let mut top = event.client_y() as f64 - offset_y;

                    if let Some(win) = web_sys::window() {
                        if let (Ok(w), Ok(h)) = (win.inner_width(), win.inner_height()) {
                            if let (Some(vw), Some(vh)) = (w.as_f64(), h.as_f64()) {
                                left = left.clamp(0.0, (vw - popup_rect.width()).max(0.0));
                                top = top.clamp(0.0, (vh - popup_rect.height()).max(0.0));
                            }
                        }
                    }

                    let style = fib_drag_move_popup.style();
                    let _ = style.set_property("left", &format!("{}px", left.round() as i32));
                    let _ = style.set_property("top", &format!("{}px", top.round() as i32));
                    let _ = style.set_property("right", "auto");
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback(
            "mousemove",
            fib_drag_move_callback.as_ref().unchecked_ref(),
        )?;
        fib_drag_move_callback.forget();

        for idx in 1..=MA_COUNT {
            let ma_enabled = doc
                .get_element_by_id(&ma_enabled_id(idx))
                .ok_or_else(|| JsValue::from_str("missing ma enabled checkbox"))?;
            let ma_period = doc
                .get_element_by_id(&ma_period_id(idx))
                .ok_or_else(|| JsValue::from_str("missing ma period input"))?;

            let ma_enabled_callback = Closure::wrap(Box::new(move || {
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            }) as Box<dyn FnMut()>);

            ma_enabled.add_event_listener_with_callback(
                "change",
                ma_enabled_callback.as_ref().unchecked_ref(),
            )?;
            ma_enabled_callback.forget();

            let ma_period_callback = Closure::wrap(Box::new(move || {
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
            }) as Box<dyn FnMut()>);

            ma_period.add_event_listener_with_callback(
                "change",
                ma_period_callback.as_ref().unchecked_ref(),
            )?;
            ma_period_callback.forget();
        }

        let settings_toggle_callback = Closure::wrap(Box::new(move || match settings_visible() {
            Ok(visible) => {
                if let Err(err) = set_settings_visible(!visible) {
                    set_status(&format!("failed to toggle settings: {:?}", err));
                }
            }
            Err(err) => {
                set_status(&format!("failed to read settings state: {:?}", err));
            }
        }) as Box<dyn FnMut()>);

        settings_toggle_button.add_event_listener_with_callback(
            "click",
            settings_toggle_callback.as_ref().unchecked_ref(),
        )?;
        settings_toggle_callback.forget();

        let settings_side_toggle_callback = Closure::wrap(Box::new(move || match settings_side() {
            Ok(current) => {
                let next = if current == "left" { "right" } else { "left" };
                if let Err(err) = set_settings_side(next) {
                    set_status(&format!("failed to move settings card: {:?}", err));
                }
            }
            Err(err) => {
                set_status(&format!("failed to read settings side: {:?}", err));
            }
        }) as Box<dyn FnMut()>);

        if let Some(settings_side_toggle_button) = settings_side_toggle_button {
            settings_side_toggle_button.add_event_listener_with_callback(
                "click",
                settings_side_toggle_callback.as_ref().unchecked_ref(),
            )?;
        }
        settings_side_toggle_callback.forget();

        let ma_drag_card = ma_settings_card.clone();
        let ma_drag_start_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            event.prevent_default();
            let rect = ma_drag_card.get_bounding_client_rect();
            let offset_x = event.client_x() as f64 - rect.left();
            let offset_y = event.client_y() as f64 - rect.top();
            MA_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = Some((offset_x, offset_y));
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        ma_settings_drag_handle.add_event_listener_with_callback(
            "mousedown",
            ma_drag_start_callback.as_ref().unchecked_ref(),
        )?;
        ma_drag_start_callback.forget();

        let ma_drag_move_card = ma_settings_card.clone();
        let ma_drag_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            MA_SETTINGS_DRAG.with(|state| {
                if let Some((offset_x, offset_y)) = *state.borrow() {
                    let card_rect = ma_drag_move_card.get_bounding_client_rect();
                    let mut left = event.client_x() as f64 - offset_x;
                    let mut top = event.client_y() as f64 - offset_y;

                    if let Some(win) = web_sys::window() {
                        if let (Ok(w), Ok(h)) = (win.inner_width(), win.inner_height()) {
                            if let (Some(vw), Some(vh)) = (w.as_f64(), h.as_f64()) {
                                left = left.clamp(0.0, (vw - card_rect.width()).max(0.0));
                                top = top.clamp(0.0, (vh - card_rect.height()).max(0.0));
                            }
                        }
                    }

                    let style = ma_drag_move_card.style();
                    let _ = style.set_property("left", &format!("{}px", left.round() as i32));
                    let _ = style.set_property("top", &format!("{}px", top.round() as i32));
                    let _ = style.set_property("right", "auto");
                    let _ = style.set_property("bottom", "auto");
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback(
            "mousemove",
            ma_drag_move_callback.as_ref().unchecked_ref(),
        )?;
        ma_drag_move_callback.forget();

        let connection_settings_toggle_callback =
            Closure::wrap(Box::new(move || match connection_settings_visible() {
                Ok(visible) => {
                    if let Err(err) = set_connection_settings_visible(!visible) {
                        set_status(&format!("failed to toggle connection settings: {:?}", err));
                    }
                }
                Err(err) => {
                    set_status(&format!(
                        "failed to read connection settings state: {:?}",
                        err
                    ));
                }
            }) as Box<dyn FnMut()>);

        connection_settings_toggle_button.add_event_listener_with_callback(
            "click",
            connection_settings_toggle_callback.as_ref().unchecked_ref(),
        )?;
        connection_settings_toggle_callback.forget();

        let connection_settings_side_toggle_callback =
            Closure::wrap(Box::new(move || match connection_settings_side() {
                Ok(current) => {
                    let next = if current == "left" { "right" } else { "left" };
                    if let Err(err) = set_connection_settings_side(next) {
                        set_status(&format!(
                            "failed to move connection settings card: {:?}",
                            err
                        ));
                    }
                }
                Err(err) => {
                    set_status(&format!(
                        "failed to read connection settings side: {:?}",
                        err
                    ));
                }
            }) as Box<dyn FnMut()>);

        if let Some(connection_settings_side_toggle_button) = connection_settings_side_toggle_button
        {
            connection_settings_side_toggle_button.add_event_listener_with_callback(
                "click",
                connection_settings_side_toggle_callback
                    .as_ref()
                    .unchecked_ref(),
            )?;
        }
        connection_settings_side_toggle_callback.forget();

        let drag_card = connection_settings_card.clone();
        let drag_start_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            event.prevent_default();
            let rect = drag_card.get_bounding_client_rect();
            let offset_x = event.client_x() as f64 - rect.left();
            let offset_y = event.client_y() as f64 - rect.top();
            CONNECTION_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = Some((offset_x, offset_y));
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        connection_settings_drag_handle.add_event_listener_with_callback(
            "mousedown",
            drag_start_callback.as_ref().unchecked_ref(),
        )?;
        drag_start_callback.forget();

        let drag_move_card = connection_settings_card.clone();
        let drag_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            CONNECTION_SETTINGS_DRAG.with(|state| {
                if let Some((offset_x, offset_y)) = *state.borrow() {
                    let card_rect = drag_move_card.get_bounding_client_rect();
                    let mut left = event.client_x() as f64 - offset_x;
                    let mut top = event.client_y() as f64 - offset_y;

                    if let Some(win) = web_sys::window() {
                        if let (Ok(w), Ok(h)) = (win.inner_width(), win.inner_height()) {
                            if let (Some(vw), Some(vh)) = (w.as_f64(), h.as_f64()) {
                                left = left.clamp(0.0, (vw - card_rect.width()).max(0.0));
                                top = top.clamp(0.0, (vh - card_rect.height()).max(0.0));
                            }
                        }
                    }

                    let style = drag_move_card.style();
                    let _ = style.set_property("left", &format!("{}px", left.round() as i32));
                    let _ = style.set_property("top", &format!("{}px", top.round() as i32));
                    let _ = style.set_property("right", "auto");
                    let _ = style.set_property("bottom", "auto");
                }
            });
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback(
            "mousemove",
            drag_move_callback.as_ref().unchecked_ref(),
        )?;
        drag_move_callback.forget();

        let drag_end_callback = Closure::wrap(Box::new(move |_event: MouseEvent| {
            MA_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            CONNECTION_SETTINGS_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            FIB_POPUP_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            CHART_DRAG.with(|state| state.borrow_mut().take());
            let stretch_finished = Y_STRETCH_DRAG.with(|state| state.borrow_mut().take()).is_some();
            if stretch_finished {
                STRETCH_TOOL_ENABLED.with(|state| {
                    *state.borrow_mut() = false;
                });
                let _ = sync_stretch_button();
                set_status("Drag mode active (default)");
            } else if STRETCH_TOOL_ENABLED.with(|state| *state.borrow()) {
                set_chart_cursor("ns-resize");
            } else {
                set_chart_cursor("grab");
            }
        }) as Box<dyn FnMut(MouseEvent)>);

        doc.add_event_listener_with_callback(
            "mouseup",
            drag_end_callback.as_ref().unchecked_ref(),
        )?;
        drag_end_callback.forget();

        let move_canvas = chart_canvas.clone();
        let mouse_move_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            show_fib_popup();
            let mut need_fib_redraw = false;
            let candles = LAST_RENDERED_CANDLES.with(|state| state.borrow().clone());
            if candles.is_empty() {
                if set_fib_preview_point(None) {
                    need_fib_redraw = true;
                }
                set_hover_info("Hover chart to see candle time");
                set_fib_popup_info("Load candles, then move cursor and click points for Fib.");
                hide_hover_tooltip();
                hide_cursor_time_label();
                hide_cursor_vline();
                hide_cursor_hline();
                hide_rsi_cursor_vline();
            } else {
                let canvas_width = move_canvas.client_width() as f64;
                let canvas_height = move_canvas.client_height() as f64;
                if canvas_width <= 0.0 || canvas_height <= 0.0 {
                    return;
                }

                let (plot_left, plot_right, plot_top, plot_bottom) =
                    match plot_bounds(canvas_width, canvas_height) {
                        Some(v) => v,
                        None => return,
                    };

                let crosshair_x = (event.offset_x() as f64).clamp(plot_left, plot_right) as i32;
                let crosshair_y = (event.offset_y() as f64).clamp(plot_top, plot_bottom) as i32;
                let canvas_rect = move_canvas.get_bounding_client_rect();
                let parent_rect = move_canvas
                    .parent_element()
                    .map(|el| el.get_bounding_client_rect());
                let canvas_left = parent_rect
                    .as_ref()
                    .map(|parent| canvas_rect.left() - parent.left())
                    .unwrap_or(0.0);
                let canvas_top = parent_rect
                    .as_ref()
                    .map(|parent| canvas_rect.top() - parent.top())
                    .unwrap_or(0.0);
                let overlay_x = (canvas_left + crosshair_x as f64).round() as i32;
                let overlay_y = (canvas_top + crosshair_y as f64).round() as i32;

                let mut is_pan_mode = false;
                PAN_LAST_X.with(|pan| {
                    let mut last = pan.borrow_mut();
                    if let Some(prev_x) = *last {
                        let dx = event.offset_x() - prev_x;
                        if dx != 0 {
                            if let Some((cur_start, cur_end)) = rendered_range() {
                                let span = (cur_end - cur_start).max(60) as f64;
                                let plot_width = (plot_right - plot_left).max(1.0);
                                let shift_seconds = (-(dx as f64) / plot_width) * span;
                                let _ = apply_panned_range_delta(
                                    cur_start,
                                    cur_end,
                                    shift_seconds,
                                    &DRAG_PAN_REMAINDER,
                                );
                            }
                        }
                        *last = Some(event.offset_x());
                        is_pan_mode = true;
                    }
                });

                if is_pan_mode {
                    if set_fib_preview_point(None) {
                        need_fib_redraw = true;
                    }
                    set_fib_popup_info("Pan mode active. Release Shift to place Fib points.");
                    hide_figure_trash();
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_cursor_hline();
                    hide_rsi_cursor_vline();
                    return;
                }

                if let Some(drag_level) = FIB_LEVEL_DRAG.with(|state| *state.borrow()) {
                    let dragged_price =
                        price_from_canvas_y(event.offset_y() as f64, plot_top, plot_bottom);
                    if let Some(price) = dragged_price {
                        show_cursor_hline(
                            crosshair_y,
                            canvas_left + plot_left,
                            canvas_left + plot_right,
                        );
                        if set_fib_level_price(drag_level, price) {
                            if let Err(err) = redraw_visible_chart_only() {
                                set_status(&format!("failed: {:?}", err));
                            }
                        }
                        let label = fib_level_drag_label(drag_level);
                        set_status(&format!("{label} moved to {:.2}", price));
                        set_fib_popup_info(&format!("Dragging {label} to {:.2}", price));
                    } else {
                        hide_cursor_hline();
                    }
                    hide_figure_trash();
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_rsi_cursor_vline();
                    set_chart_cursor("ns-resize");
                    return;
                }

                let measure_enabled = MEASURE_STATE.with(|state| state.borrow().enabled);
                let measure_anchor_a = MEASURE_STATE.with(|state| state.borrow().anchor_a);
                if measure_enabled && measure_anchor_a.is_some() {
                    if let (Some(cursor_ts), Some(cursor_price)) = (
                        timestamp_from_canvas_x(canvas_width, canvas_height, crosshair_x as f64),
                        price_from_canvas_y(crosshair_y as f64, plot_top, plot_bottom),
                    ) {
                        let previous_ts = MEASURE_DRAG_TS.with(|state| *state.borrow());
                        let previous_price = MEASURE_DRAG_PRICE.with(|state| *state.borrow());
                        let changed = previous_ts != Some(cursor_ts)
                            || previous_price
                                .map(|price| (price - cursor_price).abs() > f64::EPSILON)
                                .unwrap_or(true);
                        if changed {
                            MEASURE_DRAG_TS.with(|state| {
                                *state.borrow_mut() = Some(cursor_ts);
                            });
                            MEASURE_DRAG_PRICE.with(|state| {
                                *state.borrow_mut() = Some(cursor_price);
                            });
                        }
                        if changed {
                            if let Err(err) = redraw_visible_chart_only() {
                                set_status(&format!("failed: {:?}", err));
                            }
                        }
                        if let Some((start_ts, start_price)) = measure_anchor_a {
                            let label = format_duration_human(cursor_ts - start_ts);
                            let price_label = format_measure_price_label(start_price, cursor_price);
                            set_status(&format!(
                                "Price %: {} -> {} ({}) | price {}",
                                unix_seconds_to_hover_text(start_ts),
                                unix_seconds_to_hover_text(cursor_ts),
                                label,
                                price_label
                            ));
                            set_hover_info(&format!("Price %: {label} | price {price_label}"));
                        }
                    }
                    hide_figure_trash();
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_cursor_hline();
                    hide_rsi_cursor_vline();
                    return;
                }

                let chart_drag = CHART_DRAG.with(|state| *state.borrow());
                let mut is_chart_drag_mode = false;
                if let Some(drag) = chart_drag {
                    let dx = event.offset_x() - drag.start_x;
                    let dy = event.offset_y() - drag.start_y;
                    let plot_width = (plot_right - plot_left).max(1.0);
                    let plot_height = (plot_bottom - plot_top).max(1.0);
                    let span_x = (drag.ts_end - drag.ts_start).max(60) as f64;
                    let shift_seconds = (-(dx as f64) / plot_width * span_x).round() as i64;
                    let next_y_offset =
                        drag.y_offset_start + (dy as f64 / plot_height) * drag.y_span;
                    let y_changed = if drag.use_log_scale {
                        set_y_pan_log_offset(next_y_offset)
                    } else {
                        set_y_pan_linear_offset(next_y_offset)
                    };
                    if shift_seconds != 0 {
                        if let Err(err) = apply_range_change_client_only(
                            drag.ts_start + shift_seconds,
                            drag.ts_end + shift_seconds,
                        ) {
                            set_status(&format!("failed: {:?}", err));
                        }
                    } else if y_changed {
                        if let Err(err) = redraw_visible_chart_only() {
                            set_status(&format!("failed: {:?}", err));
                        }
                    }
                    is_chart_drag_mode = true;
                }

                if is_chart_drag_mode {
                    set_chart_cursor("grabbing");
                    hide_figure_trash();
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_cursor_hline();
                    hide_rsi_cursor_vline();
                    return;
                }

                let y_stretch_drag = Y_STRETCH_DRAG.with(|state| *state.borrow());
                let mut is_y_stretch_mode = false;
                if let Some(drag) = y_stretch_drag {
                    let dy = event.offset_y() - drag.start_y;
                    let next_factor = drag.start_factor * (dy as f64 / 180.0).exp();
                    if set_y_stretch_factor(next_factor) {
                        if let Err(err) = redraw_visible_chart_only() {
                            set_status(&format!("failed: {:?}", err));
                        }
                    }
                    is_y_stretch_mode = true;
                }

                if is_y_stretch_mode {
                    hide_figure_trash();
                    hide_hover_tooltip();
                    hide_cursor_time_label();
                    hide_cursor_vline();
                    hide_cursor_hline();
                    hide_rsi_cursor_vline();
                    return;
                }

                let cursor_ts = match timestamp_from_canvas_x(
                    canvas_width,
                    canvas_height,
                    crosshair_x as f64,
                ) {
                    Some(ts) => ts,
                    None => {
                        if set_fib_preview_point(None) {
                            need_fib_redraw = true;
                        }
                        set_hover_info("Hover chart to see candle time");
                        set_fib_popup_info("Move cursor inside chart plot area.");
                        hide_figure_trash();
                        hide_hover_tooltip();
                        hide_cursor_time_label();
                        hide_cursor_vline();
                        hide_cursor_hline();
                        hide_rsi_cursor_vline();
                        return;
                    }
                };
                let snapped_candle = nearest_candle_for_timestamp(&candles, cursor_ts);
                let text = unix_seconds_to_hover_text(cursor_ts);
                let default_price = snapped_candle
                    .map(|c| c.close)
                    .unwrap_or_else(|| candles.last().map(|c| c.close).unwrap_or(0.0));
                let usd_price = match price_from_canvas_y(crosshair_y as f64, plot_top, plot_bottom)
                {
                    Some(v) => {
                        show_cursor_hline(
                            overlay_y,
                            canvas_left + plot_left,
                            canvas_left + plot_right,
                        );
                        v
                    }
                    None => {
                        hide_cursor_hline();
                        default_price
                    }
                };

                let fib_hover_level = if !event.shift_key()
                    && !measure_enabled
                    && !STRETCH_TOOL_ENABLED.with(|state| *state.borrow())
                {
                    fib_level_hit_test(
                        crosshair_x as f64,
                        crosshair_y as f64,
                        plot_left,
                        plot_right,
                        plot_top,
                        plot_bottom,
                    )
                } else {
                    None
                };
                let fib_line_hover = fib_hover_level.is_some();
                if fib_line_hover {
                    set_chart_cursor("ns-resize");
                } else if STRETCH_TOOL_ENABLED.with(|state| *state.borrow()) {
                    set_chart_cursor("ns-resize");
                } else {
                    set_chart_cursor("grab");
                }

                // Reveal a trash icon over the figure under the cursor (only when no
                // tool is mid-use and we're not adjusting a fib level).
                let tools_active = event.shift_key()
                    || measure_enabled
                    || fib_line_hover
                    || STRETCH_TOOL_ENABLED.with(|state| *state.borrow())
                    || FIB_STATE.with(|state| state.borrow().enabled)
                    || LINE_TOOL_ENABLED.with(|state| *state.borrow());
                if tools_active {
                    hide_figure_trash();
                } else {
                    match figure_hit_test(
                        crosshair_x as f64,
                        crosshair_y as f64,
                        plot_left,
                        plot_right,
                        plot_top,
                        plot_bottom,
                    ) {
                        Some((target, fx, fy)) => {
                            cancel_figure_trash_timer();
                            FIGURE_TRASH_TARGET.with(|state| {
                                *state.borrow_mut() = Some(target);
                            });
                            position_figure_trash(canvas_left + fx, canvas_top + fy);
                        }
                        None => schedule_hide_figure_trash(),
                    }
                }

                let tooltip_text = match snapped_candle {
                    Some(candle) => format!(
                        "{} | O {:.2} H {:.2} L {:.2} C {:.2} | USD {:.2}",
                        text, candle.open, candle.high, candle.low, candle.close, usd_price
                    ),
                    None => format!("{} | USD {:.2}", text, usd_price),
                };
                let label_text = format!("{} | USD {:.2}", text, usd_price);
                let fib_text = fib_popup_text_for_cursor(cursor_ts, usd_price);
                let fib_popup_text = if let Some(level) = fib_hover_level {
                    format!(
                        "Drag {} line. Cursor price {:.2}",
                        fib_level_drag_label(level),
                        usd_price
                    )
                } else {
                    fib_text
                };
                let fib_preview = FIB_STATE.with(|fib| {
                    let cfg = *fib.borrow();
                    if cfg.enabled && cfg.draft.is_some() {
                        Some((cursor_ts, usd_price))
                    } else {
                        None
                    }
                });
                if set_fib_preview_point(fib_preview) {
                    need_fib_redraw = true;
                }

                // While the line tool has its first point placed, track the
                // cursor as the live second endpoint so the segment previews.
                let line_drawing = LINE_TOOL_ENABLED.with(|state| *state.borrow())
                    && LINE_DRAFT_ANCHOR.with(|state| state.borrow().is_some());
                let next_line_preview = if line_drawing {
                    Some((cursor_ts, usd_price))
                } else {
                    None
                };
                let line_preview_changed = LINE_PREVIEW_POINT.with(|state| {
                    let mut cur = state.borrow_mut();
                    if *cur != next_line_preview {
                        *cur = next_line_preview;
                        true
                    } else {
                        false
                    }
                });
                if line_preview_changed {
                    need_fib_redraw = true;
                }

                set_hover_info(&format!("Hover time: {} | USD {:.2}", text, usd_price));
                set_fib_popup_info(&fib_popup_text);
                show_hover_tooltip(&tooltip_text, overlay_x, overlay_y);
                show_cursor_time_label(&label_text, overlay_x);
                show_cursor_vline(overlay_x, canvas_top + plot_top, canvas_top + plot_bottom);
                show_rsi_cursor_vline(crosshair_x);
            }
            if need_fib_redraw {
                if let Err(err) = redraw_visible_chart_only() {
                    set_status(&format!("failed: {:?}", err));
                }
            }
        }) as Box<dyn FnMut(MouseEvent)>);

        chart_canvas.add_event_listener_with_callback(
            "mousemove",
            mouse_move_callback.as_ref().unchecked_ref(),
        )?;
        mouse_move_callback.forget();

        let fib_canvas = chart_canvas.clone();
        let mouse_down_callback = Closure::wrap(Box::new(move |event: MouseEvent| {
            let candles = LAST_RENDERED_CANDLES.with(|state| state.borrow().clone());
            if candles.is_empty() {
                return;
            }

            let canvas_width = fib_canvas.client_width() as f64;
            let canvas_height = fib_canvas.client_height() as f64;
            let (plot_left, plot_right, plot_top, plot_bottom) =
                match plot_bounds(canvas_width, canvas_height) {
                    Some(v) => v,
                    None => return,
                };

            if MEASURE_STATE.with(|state| state.borrow().enabled) && !event.shift_key() {
                let crosshair_x = (event.offset_x() as f64).clamp(plot_left, plot_right);
                let crosshair_y = (event.offset_y() as f64).clamp(plot_top, plot_bottom);
                let cursor_ts =
                    match timestamp_from_canvas_x(canvas_width, canvas_height, crosshair_x) {
                        Some(v) => v,
                        None => return,
                    };
                let cursor_price = match price_from_canvas_y(crosshair_y, plot_top, plot_bottom) {
                    Some(v) => v,
                    None => return,
                };
                MEASURE_STATE.with(|state| {
                    let mut cfg = state.borrow_mut();
                    cfg.anchor_a = Some((cursor_ts, cursor_price));
                    cfg.anchor_b = None;
                });
                MEASURE_DRAG_TS.with(|state| {
                    *state.borrow_mut() = Some(cursor_ts);
                });
                MEASURE_DRAG_PRICE.with(|state| {
                    *state.borrow_mut() = Some(cursor_price);
                });
                set_status(&format!(
                    "Price % start: {} @ {}. Drag to end point.",
                    unix_seconds_to_hover_text(cursor_ts),
                    format_price_label(cursor_price)
                ));
                if let Err(err) = redraw_visible_chart_only() {
                    set_status(&format!("failed: {:?}", err));
                }
                return;
            }

            let offset_x = event.offset_x() as f64;
            let offset_y = event.offset_y() as f64;
            let fib_drag_level =
                if !event.shift_key() && !STRETCH_TOOL_ENABLED.with(|state| *state.borrow()) {
                    fib_level_hit_test(offset_x, offset_y, plot_left, plot_right, plot_top, plot_bottom)
                } else {
                    None
                };
            if let Some(level) = fib_drag_level {
                FIB_LEVEL_DRAG.with(|state| {
                    *state.borrow_mut() = Some(level);
                });
                let label = fib_level_drag_label(level);
                set_chart_cursor("ns-resize");
                set_status(&format!("Dragging {label} line"));
                set_fib_popup_info(&format!("Drag {label} line to reposition it."));
                return;
            }

            if LINE_TOOL_ENABLED.with(|state| *state.borrow()) && !event.shift_key() {
                let crosshair_x = (event.offset_x() as f64).clamp(plot_left, plot_right);
                let crosshair_y = (event.offset_y() as f64).clamp(plot_top, plot_bottom);
                let cursor_ts =
                    match timestamp_from_canvas_x(canvas_width, canvas_height, crosshair_x) {
                        Some(v) => v,
                        None => return,
                    };
                let price = price_from_canvas_y(crosshair_y, plot_top, plot_bottom)
                    .unwrap_or_else(|| candles.last().map(|c| c.close).unwrap_or(0.0));

                let draft = LINE_DRAFT_ANCHOR.with(|state| *state.borrow());
                let status_message = match draft {
                    None => {
                        LINE_DRAFT_ANCHOR.with(|state| {
                            *state.borrow_mut() = Some((cursor_ts, price));
                        });
                        format!(
                            "Line first point set: {} @ {:.2}. Click second point",
                            unix_seconds_to_hover_text(cursor_ts),
                            price
                        )
                    }
                    Some(anchor) => {
                        TREND_LINES.with(|state| {
                            state.borrow_mut().push((anchor, (cursor_ts, price)));
                        });
                        persist_current_pair_drawings();
                        sync_drawings_panel();
                        LINE_DRAFT_ANCHOR.with(|state| {
                            *state.borrow_mut() = None;
                        });
                        LINE_PREVIEW_POINT.with(|state| {
                            *state.borrow_mut() = None;
                        });
                        format!(
                            "Line drawn: {} @ {:.2} -> {} @ {:.2}. Click to start another",
                            unix_seconds_to_hover_text(anchor.0),
                            anchor.1,
                            unix_seconds_to_hover_text(cursor_ts),
                            price
                        )
                    }
                };
                set_status(&status_message);
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
                return;
            }

            if FIB_STATE.with(|fib| fib.borrow().enabled) && !event.shift_key() {
                let _ = set_fib_preview_point(None);
                let crosshair_x = (event.offset_x() as f64).clamp(plot_left, plot_right);
                let crosshair_y = (event.offset_y() as f64).clamp(plot_top, plot_bottom);
                let cursor_ts =
                    match timestamp_from_canvas_x(canvas_width, canvas_height, crosshair_x) {
                        Some(v) => v,
                        None => return,
                    };
                let price = price_from_canvas_y(crosshair_y, plot_top, plot_bottom)
                    .unwrap_or_else(|| candles.last().map(|c| c.close).unwrap_or(0.0));

                // First click sets the draft anchor; the second click finishes the
                // fib, appends it, and leaves the tool on for the next one.
                let draft = FIB_STATE.with(|fib| fib.borrow().draft);
                let status_message = match draft {
                    None => {
                        FIB_STATE.with(|fib| {
                            fib.borrow_mut().draft = Some((cursor_ts, price));
                        });
                        format!(
                            "Fib first point set: {} @ {:.2}. Click second point",
                            unix_seconds_to_hover_text(cursor_ts),
                            price
                        )
                    }
                    Some(anchor) => {
                        FIB_LINES.with(|state| {
                            state.borrow_mut().push((anchor, (cursor_ts, price)));
                        });
                        // Finish the fib and switch the tool off; re-enable it to
                        // draw another.
                        FIB_STATE.with(|fib| {
                            let mut cfg = fib.borrow_mut();
                            cfg.draft = None;
                            cfg.enabled = false;
                        });
                        persist_current_pair_drawings();
                        sync_drawings_panel();
                        format!(
                            "Fib drawn: {} @ {:.2} -> {} @ {:.2}",
                            unix_seconds_to_hover_text(anchor.0),
                            anchor.1,
                            unix_seconds_to_hover_text(cursor_ts),
                            price
                        )
                    }
                };

                let fib_completed = FIB_STATE.with(|fib| !fib.borrow().enabled);
                if fib_completed {
                    if let Err(err) = sync_fib_button() {
                        set_status(&format!("failed: {:?}", err));
                        return;
                    }
                }
                set_status(&status_message);
                set_fib_popup_info(&status_message);
                spawn_local(async {
                    if let Err(err) = rerender_cached_or_fetch().await {
                        set_status(&format!("failed: {:?}", err));
                    }
                });
                return;
            }

            if event.shift_key() {
                DRAG_PAN_REMAINDER.with(|state| {
                    *state.borrow_mut() = 0.0;
                });
                PAN_LAST_X.with(|pan| {
                    *pan.borrow_mut() = Some(event.offset_x());
                });
                set_chart_cursor("grabbing");
                set_status("Pan mode: move mouse left/right");
            } else if STRETCH_TOOL_ENABLED.with(|state| *state.borrow()) {
                if offset_y >= plot_top && offset_y <= plot_bottom {
                    let start_factor = Y_STRETCH_FACTOR.with(|state| *state.borrow());
                    Y_STRETCH_DRAG.with(|state| {
                        *state.borrow_mut() = Some(YStretchDrag {
                            start_y: event.offset_y(),
                            start_factor,
                        });
                    });
                    set_chart_cursor("ns-resize");
                    set_status("Y stretch: drag up or down");
                }
            } else {
                let offset_x = event.offset_x() as f64;
                if offset_x >= plot_left
                    && offset_x <= plot_right
                    && offset_y >= plot_top
                    && offset_y <= plot_bottom
                {
                    let Some((ts_start, ts_end)) = rendered_range() else {
                        return;
                    };
                    let Some(view) = CHART_VIEW.with(|state| *state.borrow()) else {
                        return;
                    };
                    let (y_offset_start, y_span) = if view.use_log_scale {
                        let span = (view.y_high.ln() - view.y_low.ln()).max(0.000_1);
                        let offset = Y_PAN_LOG_OFFSET.with(|state| *state.borrow());
                        (offset, span)
                    } else {
                        let span = (view.y_high - view.y_low).abs().max(0.01);
                        let offset = Y_PAN_LINEAR_OFFSET.with(|state| *state.borrow());
                        (offset, span)
                    };
                    CHART_DRAG.with(|state| {
                        *state.borrow_mut() = Some(ChartDragState {
                            start_x: event.offset_x(),
                            start_y: event.offset_y(),
                            ts_start,
                            ts_end,
                            y_offset_start,
                            y_span,
                            use_log_scale: view.use_log_scale,
                        });
                    });
                    set_chart_cursor("grabbing");
                    set_status("Drag: pan X and Y");
                }
            }
        }) as Box<dyn FnMut(MouseEvent)>);

        chart_canvas.add_event_listener_with_callback(
            "mousedown",
            mouse_down_callback.as_ref().unchecked_ref(),
        )?;
        mouse_down_callback.forget();

        let mouse_up_callback = Closure::wrap(Box::new(move |_event: MouseEvent| {
            let is_pan_mode = PAN_LAST_X.with(|pan| pan.borrow().is_some());
            if is_pan_mode {
                PAN_LAST_X.with(|pan| {
                    *pan.borrow_mut() = None;
                });
                DRAG_PAN_REMAINDER.with(|state| {
                    *state.borrow_mut() = 0.0;
                });
            }
            let fib_line_drag_finished = FIB_LEVEL_DRAG.with(|state| state.borrow_mut().take());
            CHART_DRAG.with(|state| state.borrow_mut().take());
            let stretch_finished = Y_STRETCH_DRAG.with(|state| state.borrow_mut().take()).is_some();
            let measure_finished = MEASURE_STATE.with(|state| {
                let mut cfg = state.borrow_mut();
                if cfg.enabled {
                    if let (Some(end_ts), Some(end_price)) = (
                        MEASURE_DRAG_TS.with(|drag| *drag.borrow()),
                        MEASURE_DRAG_PRICE.with(|drag| *drag.borrow()),
                    ) {
                        if let Some((start_ts, start_price)) = cfg.anchor_a {
                            cfg.anchor_b = Some((end_ts, end_price));
                            cfg.enabled = false;
                            return Some((start_ts, start_price, end_ts, end_price));
                        }
                    }
                }
                None
            });
            if let Some((start_ts, start_price, end_ts, end_price)) = measure_finished {
                if let Err(err) = sync_measure_button() {
                    set_status(&format!("failed: {:?}", err));
                    return;
                }
                let label = format_duration_human(end_ts - start_ts);
                let price_label = format_measure_price_label(start_price, end_price);
                set_status(&format!(
                    "Price % measured: {} -> {} ({}) | price {}",
                    unix_seconds_to_hover_text(start_ts),
                    unix_seconds_to_hover_text(end_ts),
                    label,
                    price_label
                ));
                set_hover_info(&format!("Price %: {label} | price {price_label}"));
            }
            if stretch_finished {
                STRETCH_TOOL_ENABLED.with(|state| {
                    *state.borrow_mut() = false;
                });
                let _ = sync_stretch_button();
                set_status("Drag mode active (default)");
            } else if let Some(level) = fib_line_drag_finished {
                set_chart_cursor("grab");
                persist_current_pair_drawings();
                sync_drawings_panel();
                if let Some(price) = finished_fib_level_price(level) {
                    let label = fib_level_drag_label(level);
                    set_status(&format!("{label} fixed at {:.2}", price));
                    set_fib_popup_info(&format!("{label} moved to {:.2}", price));
                }
            } else if STRETCH_TOOL_ENABLED.with(|state| *state.borrow()) {
                set_chart_cursor("ns-resize");
            } else {
                set_chart_cursor("grab");
            }
        }) as Box<dyn FnMut(MouseEvent)>);

        chart_canvas.add_event_listener_with_callback(
            "mouseup",
            mouse_up_callback.as_ref().unchecked_ref(),
        )?;
        mouse_up_callback.forget();

        let mouse_leave_callback = Closure::wrap(Box::new(move || {
            let clear_preview = set_fib_preview_point(None);
            PAN_LAST_X.with(|pan| {
                *pan.borrow_mut() = None;
            });
            DRAG_PAN_REMAINDER.with(|state| {
                *state.borrow_mut() = 0.0;
            });
            CHART_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            FIB_LEVEL_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            Y_STRETCH_DRAG.with(|state| {
                *state.borrow_mut() = None;
            });
            set_chart_cursor("default");
            set_hover_info("Hover chart to see candle time");
            // Gentle hide so the cursor can travel onto the trash icon (which sits
            // over the canvas) without it disappearing first.
            schedule_hide_figure_trash();
            hide_hover_tooltip();
            hide_cursor_time_label();
            hide_cursor_vline();
            hide_cursor_hline();
            hide_rsi_cursor_vline();
            if clear_preview {
                if let Err(err) = redraw_visible_chart_only() {
                    set_status(&format!("failed: {:?}", err));
                }
            }
        }) as Box<dyn FnMut()>);

        chart_canvas.add_event_listener_with_callback(
            "mouseleave",
            mouse_leave_callback.as_ref().unchecked_ref(),
        )?;
        mouse_leave_callback.forget();

        Ok(())
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        setup_defaults()?;
        register_button_handler()?;
        connect_realtime_ws()?;
        spawn_local(async {
            if let Err(err) = fetch_and_draw().await {
                set_status(&format!("failed: {:?}", err));
            }
        });
        Ok(())
    }
}
