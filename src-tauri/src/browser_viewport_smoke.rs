//! Opt-in verification of our own page in an isolated, never shown application.
use super::*;
use base64::Engine;

pub async fn run(app: AppHandle) {
    let result = verify(&app).await;
    let report = match &result {
        Ok(value) => value.clone(),
        Err(error) => json!({"error":error}),
    };
    let path = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode/browser-viewport-audit/native-check.json");
    let _ = std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap());
    println!("Browser viewport: {report}");
    app.exit(if result.is_ok() { 0 } else { 1 });
}

async fn verify(app: &AppHandle) -> Result<Value, String> {
    let tab = ensure_tab(app, "viewport-fixture", "main").await?;
    let mount = "viewport-fixture".to_string();
    *tab.mount.lock().unwrap() = mount.clone();
    let owner = app.get_webview("main").ok_or("Missing isolated owner")?;
    if owner.window().is_visible().map_err(|e| e.to_string())? {
        return Err("Isolated owner must stay hidden".into());
    }
    // No desktop input, focus calls or external websites. The application stays hidden.
    app.get_webview(&tab.label).ok_or("Missing page")?.eval("document.body.innerHTML='<style>body{margin:0;height:2000px;background:#dcf1ff}button{position:absolute;left:1130px;top:680px;width:120px;height:70px;background:#e23a2e;border:0}</style><button onclick=\"document.body.dataset.clicks=Number(document.body.dataset.clicks||0)+1\">Right edge</button>'; window.fixtureId=String(Math.random()); window.ticks=0; setInterval(()=>window.ticks++,50)").map_err(|e|e.to_string())?;
    let lease = Lease {
        session: tab.session.clone(),
        agent: "fixture".into(),
        turn: None,
        gate: Arc::new(AtomicBool::new(true)),
        presented: Arc::new(AtomicBool::new(true)),
    };
    let mut results = Vec::new();
    let mut page_id = String::new();
    let mut previous_snapshot: Option<Value> = None;
    for width in [380., 240., 1000., 380.] {
        browser_layout(
            tab.session.clone(),
            mount.clone(),
            5.,
            10.,
            width,
            650.,
            true,
            owner.clone(),
            app.clone(),
        )
        .await?;
        if let Some(id) = previous_snapshot.as_ref() {
            observation(&tab, id)?;
        }
        let observed = snapshot(app, &tab, &lease).await?;
        let data: Value = serde_json::from_str(
            observed["content"][0]["text"]
                .as_str()
                .ok_or("Missing snapshot")?,
        )
        .map_err(|e| e.to_string())?;
        previous_snapshot = Some(data["snapshotId"].clone());
        if data["width"] != 1280 || data["height"] != 800 {
            return Err(format!("Viewport follows panel: {data}"));
        }
        let image = observed["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "image")
            .ok_or("No screenshot")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        let picture = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        if (picture.width(), picture.height()) != (1280, 800) {
            return Err(format!(
                "Cropped screenshot: {}x{}",
                picture.width(),
                picture.height()
            ));
        }
        let preview = native_preview(app, &tab).await?;
        let rendered = image::load_from_memory(&preview)
            .map_err(|e| e.to_string())?
            .to_rgb8();
        std::fs::write(
            ".supercode/browser-viewport-audit/preview-latest.png",
            &preview,
        )
        .map_err(|e| e.to_string())?;
        let marker = rendered
            .get_pixel(
                (rendered.width() as f64 * 1150. / 1280.) as u32,
                (rendered.height() as f64 * 695. / 800.) as u32,
            )
            .0;
        if marker[0] < 150 || marker[1] > 140 || marker[2] > 140 {
            return Err(format!(
                "Native preview crops the right edge: {}x{}, marker={marker:?}",
                rendered.width(),
                rendered.height()
            ));
        }
        if width == 380. {
            std::fs::write(
                ".supercode/browser-viewport-audit/preview-380.png",
                &preview,
            )
            .map_err(|e| e.to_string())?;
        }
        let identity = evaluate(
            app,
            &tab,
            "({id:fixtureId,ticks,clicks:Number(document.body.dataset.clicks||0)})".into(),
            None,
            None,
        )
        .await?;
        if page_id.is_empty() {
            page_id = identity["id"].as_str().ok_or("Fixture not ready")?.into();
        }
        if identity["id"] != page_id {
            return Err("Page was replaced".into());
        }
        results.push(json!({"panelWidth":width,"viewport":[data["width"],data["height"]],"screenshot":[picture.width(),picture.height()],"nativePreview":[rendered.width(),rendered.height()],"rightEdgeMarker":marker,"rect":*tab.rect.lock().unwrap()}));
    }
    let before = evaluate(app, &tab, "ticks".into(), None, None).await?;
    browser_close(
        tab.session.clone(),
        mount.clone(),
        owner.clone(),
        app.clone(),
    )
    .await?;
    if !lease.gate.load(Ordering::Acquire) || tab.visible.load(Ordering::Acquire) {
        return Err("Closing preview stopped the browser".into());
    }
    let mut advanced = false;
    for _ in 0..22 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if evaluate(app, &tab, "ticks".into(), None, None)
            .await?
            .as_u64()
            > before.as_u64()
        {
            advanced = true;
            break;
        }
    }
    if !advanced {
        return Err("Hidden page is suspended".into());
    }
    let hidden = snapshot(app, &tab, &lease).await?;
    if !hidden["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "image")
    {
        return Err("Hidden screenshot missing".into());
    }
    let hidden_data: Value = serde_json::from_str(
        hidden["content"][0]["text"]
            .as_str()
            .ok_or("Missing hidden snapshot")?,
    )
    .map_err(|e| e.to_string())?;
    let target = hidden_data["elements"]
        .as_array()
        .ok_or("Missing hidden controls")?
        .iter()
        .find(|el| el["name"] == "Right edge")
        .ok_or("Right edge is outside the viewport")?;
    let result = execute(
        app,
        &tab,
        &lease,
        "browser_click",
        json!({"snapshotId":hidden_data["snapshotId"],"ref":target["ref"]}),
    )
    .await?;
    if result["isError"] == true {
        return Err(format!("Hidden click failed: {result}"));
    }
    let clicked = evaluate(
        app,
        &tab,
        "Number(document.body.dataset.clicks||0)".into(),
        None,
        None,
    )
    .await?;
    if clicked != 1 {
        return Err(format!("Scaled click missed: {clicked}"));
    }
    let clicked_data: Value = serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .ok_or("Missing post-click snapshot")?,
    )
    .map_err(|e| e.to_string())?;
    let scrolled = execute(
        app,
        &tab,
        &lease,
        "browser_scroll",
        json!({"snapshotId":clicked_data["snapshotId"],"deltaY":400}),
    )
    .await?;
    if scrolled["isError"] == true {
        return Err(format!("Hidden scroll failed: {scrolled}"));
    }
    let scrolled_data: Value = serde_json::from_str(
        scrolled["content"][0]["text"]
            .as_str()
            .ok_or("Missing scrolled snapshot")?,
    )
    .map_err(|e| e.to_string())?;
    if scrolled_data["scrollY"] != 400 {
        return Err(format!("Scroll not retained: {scrolled_data}"));
    }
    let frame = scrolled["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "image")
        .ok_or("Missing scrolled screenshot")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(frame["data"].as_str().unwrap())
        .map_err(|e| e.to_string())?;
    let pixels = image::load_from_memory(&bytes)
        .map_err(|e| e.to_string())?
        .to_rgb8();
    let marker = pixels.get_pixel(1150, 295).0;
    if marker[0] < 150 || marker[1] > 140 || marker[2] > 140 {
        return Err(format!("Screenshot lost scroll position: {marker:?}"));
    }
    browser_layout(
        tab.session.clone(),
        mount,
        5.,
        10.,
        380.,
        650.,
        true,
        owner,
        app.clone(),
    )
    .await?;
    let native_position = app
        .get_webview(&tab.label)
        .ok_or("Missing reopened view")?
        .position()
        .map_err(|e| e.to_string())?;
    let dpi = f64::from_bits(tab.scale.load(Ordering::Acquire));
    let rect = *tab.rect.lock().unwrap();
    if (native_position.x as f64 - rect.0 * dpi).abs() > 2.
        || (native_position.y as f64 - rect.1 * dpi).abs() > 2.
    {
        return Err(format!(
            "Reopened preview is still parked: {native_position:?}"
        ));
    }
    if evaluate(app, &tab, "fixtureId".into(), None, None).await? != page_id {
        return Err("Reopening replaced page".into());
    }
    if app
        .get_window("main")
        .ok_or("Missing owner")?
        .is_visible()
        .map_err(|e| e.to_string())?
    {
        return Err("Isolated owner was shown".into());
    }
    Ok(
        json!({"cases":results,"hiddenPageContinues":true,"hiddenScreenshot":true,"rightEdgeClick":true,"scrolledScreenshot":true,"samePageOnReopen":true,"restoredNativePosition":true,"mainWindowNeverShown":true}),
    )
}

#[cfg(windows)]
async fn native_preview(app: &AppHandle, tab: &Tab) -> Result<Vec<u8>, String> {
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::{
        core::Interface,
        Win32::System::Com::{IStream, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET},
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let completed = sender.clone();
    app.get_webview(&tab.label)
        .ok_or("Missing preview")?
        .with_webview(move |platform| unsafe {
            let result = (|| -> windows::core::Result<()> {
                let raw = windows_sys::Win32::UI::Shell::SHCreateMemStream(std::ptr::null(), 0);
                if raw.is_null() {
                    return Err(windows::core::Error::from_thread());
                }
                let stream = IStream::from_raw(raw);
                let output = stream.clone();
                let handler = CapturePreviewCompletedHandler::create(Box::new(move |error| {
                    let bytes = (|| -> windows::core::Result<Vec<u8>> {
                        error?;
                        let mut info = STATSTG::default();
                        output.Stat(&mut info, STATFLAG_NONAME)?;
                        output.Seek(0, STREAM_SEEK_SET, None)?;
                        let mut bytes = vec![0u8; info.cbSize as usize];
                        let mut count = 0;
                        output
                            .Read(
                                bytes.as_mut_ptr() as _,
                                bytes.len() as u32,
                                Some(&mut count),
                            )
                            .ok()?;
                        bytes.truncate(count as usize);
                        Ok(bytes)
                    })()
                    .map_err(|error| error.to_string());
                    if let Some(tx) = completed.lock().unwrap().take() {
                        let _ = tx.send(bytes);
                    }
                    Ok(())
                }));
                platform.controller().CoreWebView2()?.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
            })();
            if let Err(error) = result {
                if let Some(tx) = sender.lock().unwrap().take() {
                    let _ = tx.send(Err(error.to_string()));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(8), rx)
        .await
        .map_err(|_| "Native preview timed out".to_string())?
        .map_err(|e| e.to_string())?
}
#[cfg(not(windows))]
async fn native_preview(_: &AppHandle, _: &Tab) -> Result<Vec<u8>, String> {
    Err("Windows only".into())
}
