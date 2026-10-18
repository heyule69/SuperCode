//! The page has one desktop viewport; the panel only scales its presentation.
use serde_json::{json, Value};

pub const WIDTH: f64 = 1280.;
pub const HEIGHT: f64 = 800.;

pub fn fit(x: f64, y: f64, width: f64, height: f64, dpi: f64) -> (f64, f64, f64, f64) {
    // An 8:5 physical-pixel rectangle avoids aspect-ratio drift at fractional DPI.
    let units = ((width * dpi / 8.).min(height * dpi / 5.)).floor().max(1.);
    let w = units * 8. / dpi;
    let h = units * 5. / dpi;
    (x + (width - w) / 2., y, w, h)
}

pub fn metrics(width: f64) -> Value {
    // CDP uses logical host units; WebView2 applies monitor rasterization later.
    json!({"width":1280,"height":800,"deviceScaleFactor":1,"mobile":false,
        "screenWidth":1280,"screenHeight":800,"scale":width/WIDTH,
        "dontSetVisibleSize":true})
}

pub fn screenshot(page_x: f64, page_y: f64) -> Value {
    json!({"format":"jpeg","quality":65,"fromSurface":true,"captureBeyondViewport":true,
        "clip":{"x":page_x,"y":page_y,"width":WIDTH,"height":HEIGHT,"scale":1}})
}

pub fn input_point(point: &Value, scale: f64) -> Value {
    // Input is dispatched into WebView2's host area, while DOM observations
    // and visual cues use the independent page's CSS coordinates.
    json!({"x":point["x"].as_f64().map(|x|x*scale),
        "y":point["y"].as_f64().map(|y|y*scale)})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn narrow_wide_and_fractional_dpi_previews_keep_the_same_desktop_viewport() {
        for dpi in [1., 1.25, 1.5, 2.] {
            for (width, height) in [(240., 750.), (380., 900.), (1500., 600.)] {
                let (x, y, w, h) = fit(20., 30., width, height, dpi);
                assert!(
                    x >= 20.
                        && y >= 30.
                        && x + w <= 20. + width + 0.001
                        && y + h <= 30. + height + 0.001
                );
                assert!((w / h - WIDTH / HEIGHT).abs() < 0.0001);
                let args = metrics(w);
                assert_eq!(args["width"], 1280);
                assert_eq!(args["height"], 800);
                assert_eq!(args["deviceScaleFactor"], 1);
                assert!((WIDTH * args["scale"].as_f64().unwrap() - w).abs() < 0.0001);
            }
        }
    }
    #[test]
    fn screenshots_capture_the_complete_scrolled_viewport_at_original_resolution() {
        let args = screenshot(0., 375.);
        assert_eq!(
            args["clip"],
            json!({"x":0.,"y":375.,"width":1280.,"height":800.,"scale":1})
        );
        assert_eq!(args["captureBeyondViewport"], true);
    }
    #[test]
    fn input_maps_virtual_page_points_into_the_scaled_host_area() {
        assert_eq!(
            input_point(&json!({"x":1190,"y":715}), 380. / 1280.),
            json!({"x":353.28125,"y":212.265625})
        );
        assert_eq!(
            input_point(&json!({"x":1190,"y":715}), 1.),
            json!({"x":1190.,"y":715.})
        );
        assert_eq!(
            input_point(&json!({"x":"invalid"}), 0.25),
            json!({"x":null,"y":null})
        );
    }
}
