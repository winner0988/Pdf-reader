//! POC for ADR 0030 (accepted 2026-11-21, #686): creating form fields with MuPDF.
//!
//! Evaluates whether form fields (text, multiline, password, checkbox, radio group,
//! combo dropdown, listbox, signature) can be created using existing MuPDF bindings,
//! appearances generated properly, Chinese characters supported, tab order set,
//! and forbidden script/action keys avoided.
//!
//! This file is a POC and is NOT merged into `main`.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Once;

use mupdf::pdf::{
    AnnotationFlags, FieldFlags, PdfAnnotationType, PdfDocument as MuPdf, PdfObject,
    PdfWriteOptions,
};
use mupdf::{Buffer, Image, Rect, Size};
use pdf_worker::engine::PdfDocument;

#[allow(dead_code)]
mod common;
use common::check_qpdf_bytes;

/// Returns the AcroForm `/Fields` array, creating `/AcroForm` if needed.
fn get_or_create_acroform_fields(doc: &mut MuPdf) -> PdfObject {
    let trailer = doc.trailer().expect("trailer");
    let mut root = trailer.get_dict("Root").expect("Root").expect("Root dict");
    let mut acroform = match root.get_dict("AcroForm").expect("get AcroForm") {
        Some(af) => af,
        None => {
            let af = doc.new_dict().expect("new dict");
            root.dict_put("AcroForm", af).expect("put AcroForm");
            root.get_dict("AcroForm")
                .expect("get AcroForm")
                .expect("AcroForm dict")
        }
    };
    match acroform.get_dict("Fields").expect("get Fields") {
        Some(f) => f,
        None => {
            let f = doc.new_array().expect("new array");
            acroform.dict_put("Fields", f).expect("put Fields");
            acroform
                .get_dict("Fields")
                .expect("get Fields")
                .expect("Fields array")
        }
    }
}

/// Returns the directory for POC output samples: `<CARGO_TARGET_TMPDIR>/poc-686/`
fn get_poc_output_dir() -> PathBuf {
    let base = if let Ok(target_tmp) = std::env::var("CARGO_TARGET_TMPDIR") {
        PathBuf::from(target_tmp)
    } else if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        PathBuf::from(target_dir).join("tmp")
    } else {
        PathBuf::from("target/tmp")
    };
    let dir = base.join("poc-686");
    fs::create_dir_all(&dir).expect("create poc-686 dir");
    dir
}

/// Security audit: walks all objects in `1..=count_objects()` recursively.
///
/// Ensures:
/// 1. No forbidden actions or scripts: `/A`, `/AA`, `/JS`, `/JavaScript`, `/CO`,
///    `/XFA`, `/NeedAppearances`, `/RV`, `/DS`.
/// 2. If `/FT /Sig`, no `/V` or `/SV`.
/// 3. Returns `Err` if any forbidden key is detected.
fn audit_pdf_security(doc: &MuPdf) -> Result<(), String> {
    let count = doc.count_objects().map_err(|e| e.to_string())?;
    let mut visited = HashSet::new();
    for i in 1..=count {
        if let Ok(Some(obj)) = doc.xref_object(i as i32) {
            check_obj_security(&obj, &mut visited)?;
        }
    }
    Ok(())
}

#[allow(clippy::collapsible_if)]
fn check_obj_security(obj: &PdfObject, visited: &mut HashSet<i32>) -> Result<(), String> {
    if let Ok(indir) = obj.as_indirect() {
        if indir > 0 && !visited.insert(indir) {
            return Ok(());
        }
    }

    if obj.is_dict().unwrap_or(false) {
        let len = obj.dict_len().unwrap_or(0);
        for idx in 0..len {
            if let Ok(Some(k)) = obj.get_dict_key(idx as i32) {
                if let Ok(name) = k.as_name() {
                    let key_str = String::from_utf8_lossy(&name);
                    let forbidden = [
                        "A",
                        "AA",
                        "JS",
                        "JavaScript",
                        "CO",
                        "XFA",
                        "NeedAppearances",
                        "RV",
                        "DS",
                    ];
                    if forbidden.contains(&key_str.as_ref()) {
                        return Err(format!("Forbidden key found in PDF object: /{key_str}"));
                    }
                    if key_str == "FT" {
                        if let Ok(Some(val)) = obj.get_dict_val(idx as i32) {
                            if val.as_name().map(|n| n == b"Sig").unwrap_or(false) {
                                if obj.get_dict("V").ok().flatten().is_some() {
                                    return Err("Signature field must not contain /V".into());
                                }
                                if obj.get_dict("SV").ok().flatten().is_some() {
                                    return Err("Signature field must not contain /SV".into());
                                }
                            }
                        }
                    }
                }
            }
            if let Ok(Some(v)) = obj.get_dict_val(idx as i32) {
                check_obj_security(&v, visited)?;
            }
        }
    } else if obj.is_array().unwrap_or(false) {
        let len = obj.len().unwrap_or(0);
        for idx in 0..len {
            if let Ok(Some(item)) = obj.get_array(idx as i32) {
                check_obj_security(&item, visited)?;
            }
        }
    }
    Ok(())
}

/// Helper: creates a custom border-only Form XObject appearance stream for a signature field.
fn create_signature_border_appearance(
    doc: &mut MuPdf,
    width: f32,
    height: f32,
) -> Result<PdfObject, mupdf::Error> {
    let stream_content = format!(
        "q 1 1 {:.1} {:.1} re 1 w 0.2 0.4 0.8 RG S Q\n",
        width - 2.0,
        height - 2.0
    );
    let buf = Buffer::from_bytes(stream_content.as_bytes())?;
    let mut dict = doc.new_dict()?;
    dict.dict_put("Type", PdfObject::new_name("XObject")?)?;
    dict.dict_put("Subtype", PdfObject::new_name("Form")?)?;
    dict.dict_put("FormType", PdfObject::new_int(1)?)?;
    let mut bbox = doc.new_array()?;
    bbox.array_push(PdfObject::new_real(0.0)?)?;
    bbox.array_push(PdfObject::new_real(0.0)?)?;
    bbox.array_push(PdfObject::new_real(width)?)?;
    bbox.array_push(PdfObject::new_real(height)?)?;
    dict.dict_put("BBox", bbox)?;

    doc.add_stream(&buf, Some(&dict), false)
}

// ============================================================================
// Item 1: Field Creation and Appearance Generation
// ============================================================================
#[test]
fn test_item1_field_creation_and_appearance_keys() {
    let mut doc = MuPdf::new();
    let mut page = doc.new_page(Size::A4).expect("new page");
    let mut fields = get_or_create_acroform_fields(&mut doc);

    // 1. Text field (single-line)
    let mut text_field = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("text");
    let mut tf_obj = text_field.object();
    tf_obj
        .dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
        .expect("FT");
    tf_obj
        .dict_put("T", PdfObject::new_string("SingleLineText").expect("T"))
        .expect("T");
    tf_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    tf_obj
        .dict_put("V", PdfObject::new_string("Hello World").expect("V"))
        .expect("V");
    text_field
        .set_rect(Rect::new(50.0, 750.0, 250.0, 780.0))
        .expect("rect");
    text_field
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(text_field.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 2. Multiline text field
    let mut multiline = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("multiline");
    let mut ml_obj = multiline.object();
    ml_obj
        .dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
        .expect("FT");
    ml_obj
        .dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::MULTILINE.bits()).expect("Ff"),
        )
        .expect("Ff");
    ml_obj
        .dict_put("T", PdfObject::new_string("MultilineText").expect("T"))
        .expect("T");
    ml_obj
        .dict_put("DA", PdfObject::new_string("/Helv 10 Tf 0 g").expect("DA"))
        .expect("DA");
    ml_obj
        .dict_put(
            "V",
            PdfObject::new_string("Line 1\nLine 2\nLine 3").expect("V"),
        )
        .expect("V");
    multiline
        .set_rect(Rect::new(50.0, 680.0, 250.0, 740.0))
        .expect("rect");
    multiline
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(multiline.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 3. Password field
    let mut password = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("password");
    let mut pw_obj = password.object();
    pw_obj
        .dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
        .expect("FT");
    pw_obj
        .dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::PASSWORD.bits()).expect("Ff"),
        )
        .expect("Ff");
    pw_obj
        .dict_put("T", PdfObject::new_string("PasswordField").expect("T"))
        .expect("T");
    pw_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    pw_obj
        .dict_put("V", PdfObject::new_string("Secret123").expect("V"))
        .expect("V");
    password
        .set_rect(Rect::new(50.0, 640.0, 250.0, 670.0))
        .expect("rect");
    password
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(password.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 4. Checkbox
    let mut chk = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("chk");
    let mut chk_obj = chk.object();
    chk_obj
        .dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
        .expect("FT");
    chk_obj
        .dict_put("T", PdfObject::new_string("OptInCheck").expect("T"))
        .expect("T");
    chk_obj
        .dict_put("AS", PdfObject::new_name("Yes").expect("AS"))
        .expect("AS");
    chk_obj
        .dict_put("V", PdfObject::new_name("Yes").expect("V"))
        .expect("V");
    chk.set_rect(Rect::new(50.0, 600.0, 70.0, 620.0))
        .expect("rect");
    chk.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
    fields
        .array_push(
            doc.new_indirect(chk.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 5. Radio button group (2 radio buttons)
    let mut radio_parent = doc.new_dict().expect("radio_parent");
    radio_parent
        .dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
        .expect("FT");
    radio_parent
        .dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::RADIO.bits() | FieldFlags::NO_TOGGLE_TO_OFF.bits())
                .expect("Ff"),
        )
        .expect("Ff");
    radio_parent
        .dict_put("T", PdfObject::new_string("RadioDelivery").expect("T"))
        .expect("T");
    radio_parent
        .dict_put("V", PdfObject::new_name("Express").expect("V"))
        .expect("V");
    let mut radio_kids = doc.new_array().expect("radio_kids");

    let mut r1 = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("r1");
    let mut r1_obj = r1.object();
    r1_obj
        .dict_put("AS", PdfObject::new_name("Standard").expect("AS"))
        .expect("AS");
    r1.set_rect(Rect::new(50.0, 560.0, 70.0, 580.0))
        .expect("rect");
    r1.set_flags(AnnotationFlags::IS_PRINT).expect("flags");

    let mut r2 = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("r2");
    let mut r2_obj = r2.object();
    r2_obj
        .dict_put("AS", PdfObject::new_name("Express").expect("AS"))
        .expect("AS");
    r2.set_rect(Rect::new(100.0, 560.0, 120.0, 580.0))
        .expect("rect");
    r2.set_flags(AnnotationFlags::IS_PRINT).expect("flags");

    let r1_indir = doc
        .new_indirect(r1.xref().expect("xref"), 0)
        .expect("r1_indir");
    let r2_indir = doc
        .new_indirect(r2.xref().expect("xref"), 0)
        .expect("r2_indir");
    radio_kids.array_push(r1_indir).expect("push r1");
    radio_kids.array_push(r2_indir).expect("push r2");
    radio_parent.dict_put("Kids", radio_kids).expect("put Kids");

    let r_parent_obj = doc.create_object().expect("create parent obj");
    let r_parent_num = r_parent_obj.as_indirect().expect("parent indirect num");
    let mut r_parent_indirect = doc.new_indirect(r_parent_num, 0).expect("parent indir");
    r1_obj
        .dict_put("Parent", doc.new_indirect(r_parent_num, 0).expect("parent"))
        .expect("put parent 1");
    r2_obj
        .dict_put("Parent", doc.new_indirect(r_parent_num, 0).expect("parent"))
        .expect("put parent 2");
    r_parent_indirect
        .write_object(&radio_parent)
        .expect("write parent");
    fields.array_push(r_parent_indirect).expect("push radio");

    // 6. Combo dropdown
    let mut combo = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("combo");
    let mut cb_obj = combo.object();
    cb_obj
        .dict_put("FT", PdfObject::new_name("Ch").expect("FT"))
        .expect("FT");
    cb_obj
        .dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::COMBO.bits()).expect("Ff"),
        )
        .expect("Ff");
    cb_obj
        .dict_put("T", PdfObject::new_string("ComboLanguage").expect("T"))
        .expect("T");
    cb_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    let mut c_opts = doc.new_array().expect("c_opts");
    c_opts
        .array_push(PdfObject::new_string("English").expect("s"))
        .expect("push");
    c_opts
        .array_push(PdfObject::new_string("繁體中文").expect("s"))
        .expect("push");
    cb_obj.dict_put("Opt", c_opts).expect("put Opt");
    cb_obj
        .dict_put("V", PdfObject::new_string("English").expect("V"))
        .expect("V");
    combo
        .set_rect(Rect::new(50.0, 510.0, 200.0, 540.0))
        .expect("rect");
    combo.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
    fields
        .array_push(
            doc.new_indirect(combo.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 7. Listbox
    let mut list = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("list");
    let mut lb_obj = list.object();
    lb_obj
        .dict_put("FT", PdfObject::new_name("Ch").expect("FT"))
        .expect("FT");
    lb_obj
        .dict_put("T", PdfObject::new_string("ListPriority").expect("T"))
        .expect("T");
    lb_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    let mut l_opts = doc.new_array().expect("l_opts");
    l_opts
        .array_push(PdfObject::new_string("Low").expect("s"))
        .expect("push");
    l_opts
        .array_push(PdfObject::new_string("Normal").expect("s"))
        .expect("push");
    l_opts
        .array_push(PdfObject::new_string("High").expect("s"))
        .expect("push");
    lb_obj.dict_put("Opt", l_opts).expect("put Opt");
    lb_obj
        .dict_put("V", PdfObject::new_string("High").expect("V"))
        .expect("V");
    list.set_rect(Rect::new(50.0, 420.0, 200.0, 490.0))
        .expect("rect");
    list.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
    fields
        .array_push(
            doc.new_indirect(list.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 8. Signature field
    let sig_widget = page.add_signature_widget("SignatureField").expect("sig");
    let mut sig_annot = sig_widget.into_annotation();
    sig_annot
        .set_rect(Rect::new(50.0, 330.0, 250.0, 400.0))
        .expect("sig rect");
    sig_annot
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("sig flags");

    // Check keys BEFORE appearance generation:
    assert!(
        tf_obj.get_dict("AP").unwrap().is_none(),
        "AP must be absent before update"
    );
    let mut sig_obj = sig_annot.object();
    let lock_before = sig_obj.get_dict("Lock").unwrap();
    println!("Signature /Lock before update: {:?}", lock_before.is_some());
    // Delete /Lock if present, without FFI
    if lock_before.is_some() {
        sig_obj.dict_delete("Lock").expect("delete Lock");
    }
    assert!(
        sig_obj.get_dict("Lock").unwrap().is_none(),
        "Lock must be removed"
    );

    // Update appearances using widget.update()
    let mut update_count = 0;
    for mut w in page.widgets() {
        let updated = w.update().expect("update widget");
        if updated {
            update_count += 1;
        }
    }
    println!("Updated widgets count: {update_count}");
    assert!(
        update_count >= 8,
        "All 8 widget instances must report appearance updated"
    );

    // Check keys AFTER appearance update:
    // Text field must now have /AP /N stream
    let ap_tf = tf_obj.get_dict("AP").expect("get AP").expect("AP dict");
    let ap_n_tf = ap_tf.get_dict("N").expect("get N").expect("N stream");
    assert!(
        ap_n_tf.is_stream().expect("is_stream"),
        "/AP /N must be a stream"
    );
    let tf_stream = ap_n_tf.read_stream().expect("read stream");
    assert!(
        String::from_utf8_lossy(&tf_stream).contains("(Hello World) Tj"),
        "Text appearance stream must draw value"
    );

    // Checkbox must have /AP /N subdictionary with /Yes and /Off
    let ap_chk = chk_obj.get_dict("AP").expect("get AP").expect("AP dict");
    let ap_n_chk = ap_chk.get_dict("N").expect("get N").expect("N dict");
    assert!(
        ap_n_chk.is_dict().expect("is_dict"),
        "Checkbox /AP /N must be a dictionary"
    );
    assert!(
        ap_n_chk.get_dict("Yes").expect("get Yes").is_some(),
        "Checkbox must have Yes state"
    );
    assert!(
        ap_n_chk.get_dict("Off").expect("get Off").is_some(),
        "Checkbox must have Off state"
    );

    // Signature field: verify it is attached, unsigned, has no /V or /SV
    assert!(
        sig_obj.get_dict("V").unwrap().is_none(),
        "Signature must not have /V"
    );
    assert!(
        sig_obj.get_dict("SV").unwrap().is_none(),
        "Signature must not have /SV"
    );

    // Verify unselected radio button is reset to Off after appearance update
    r1_obj
        .dict_put("AS", PdfObject::new_name("Off").expect("Off"))
        .expect("put Off");

    let mut pdf_bytes = Vec::new();
    let mut write_opts = PdfWriteOptions::default();
    write_opts.set_appearance(true);
    doc.write_to_with_options(&mut pdf_bytes, write_opts)
        .expect("write_to");
    check_qpdf_bytes(&pdf_bytes);

    let engine_doc = PdfDocument::from_bytes(&pdf_bytes).expect("open engine doc");
    let pf = engine_doc.page_fields(0).expect("page_fields");
    assert_eq!(
        pf.len(),
        8,
        "Expected 8 fields (single, multi, pw, chk, radio, combo, list, sig)"
    );
    println!("Item 1: All 8 field types created and verified with qpdf exit code 0.");
}

// ============================================================================
// Item 2: Button State Names and Ordering
// ============================================================================
#[test]
fn test_item2_button_state_names_and_ordering() {
    let mut doc = MuPdf::new();
    let mut page = doc.new_page(Size::A4).expect("new page");
    let mut fields = get_or_create_acroform_fields(&mut doc);

    // Part A: ASCII buttons with proper ordering (set /AS before update)
    // 1. Checkbox with export value "Agree"
    let mut chk_agree = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("chk_agree");
    let mut ca_obj = chk_agree.object();
    ca_obj
        .dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
        .expect("FT");
    ca_obj
        .dict_put("T", PdfObject::new_string("AgreeBox").expect("T"))
        .expect("T");
    ca_obj
        .dict_put("TU", PdfObject::new_string("Terms Agreement").expect("TU"))
        .expect("TU");
    ca_obj
        .dict_put("AS", PdfObject::new_name("Agree").expect("AS"))
        .expect("AS");
    ca_obj
        .dict_put("V", PdfObject::new_name("Agree").expect("V"))
        .expect("V");
    chk_agree
        .set_rect(Rect::new(50.0, 750.0, 70.0, 770.0))
        .expect("rect");
    chk_agree
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(chk_agree.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 2. Radio group with export values "Small" and "Large"
    let mut radio_parent = doc.new_dict().expect("radio_parent");
    radio_parent
        .dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
        .expect("FT");
    radio_parent
        .dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::RADIO.bits() | FieldFlags::NO_TOGGLE_TO_OFF.bits())
                .expect("Ff"),
        )
        .expect("Ff");
    radio_parent
        .dict_put("T", PdfObject::new_string("SizeRadio").expect("T"))
        .expect("T");
    radio_parent
        .dict_put("V", PdfObject::new_name("Small").expect("V"))
        .expect("V");
    let mut r_kids = doc.new_array().expect("r_kids");

    let mut r_small = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("r_small");
    let mut rs_obj = r_small.object();
    rs_obj
        .dict_put("AS", PdfObject::new_name("Small").expect("AS"))
        .expect("AS");
    r_small
        .set_rect(Rect::new(50.0, 700.0, 70.0, 720.0))
        .expect("rect");
    r_small.set_flags(AnnotationFlags::IS_PRINT).expect("flags");

    let mut r_large = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("r_large");
    let mut rl_obj = r_large.object();
    // Temporarily set AS to Large so MuPDF creates the Large ON state
    rl_obj
        .dict_put("AS", PdfObject::new_name("Large").expect("AS"))
        .expect("AS");
    r_large
        .set_rect(Rect::new(100.0, 700.0, 120.0, 720.0))
        .expect("rect");
    r_large.set_flags(AnnotationFlags::IS_PRINT).expect("flags");

    r_kids
        .array_push(
            doc.new_indirect(r_small.xref().expect("xref"), 0)
                .expect("indir"),
        )
        .expect("push");
    r_kids
        .array_push(
            doc.new_indirect(r_large.xref().expect("xref"), 0)
                .expect("indir"),
        )
        .expect("push");
    radio_parent.dict_put("Kids", r_kids).expect("put Kids");

    let r_p_obj = doc.create_object().expect("parent obj");
    let r_p_num = r_p_obj.as_indirect().expect("parent indirect num");
    let mut r_p_indirect = doc.new_indirect(r_p_num, 0).expect("parent indir");
    rs_obj
        .dict_put("Parent", doc.new_indirect(r_p_num, 0).expect("p"))
        .expect("put parent");
    rl_obj
        .dict_put("Parent", doc.new_indirect(r_p_num, 0).expect("p"))
        .expect("put parent");
    r_p_indirect
        .write_object(&radio_parent)
        .expect("write parent");
    fields.array_push(r_p_indirect).expect("push radio");

    // Part B: Chinese export values
    let mut chk_chinese = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("chk_chinese");
    let mut cc_obj = chk_chinese.object();
    cc_obj
        .dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
        .expect("FT");
    cc_obj
        .dict_put("T", PdfObject::new_string("ChineseAgree").expect("T"))
        .expect("T");
    cc_obj
        .dict_put("TU", PdfObject::new_string("同意授權").expect("TU"))
        .expect("TU");
    cc_obj
        .dict_put("AS", PdfObject::new_name("同意").expect("AS"))
        .expect("AS");
    cc_obj
        .dict_put("V", PdfObject::new_name("同意").expect("V"))
        .expect("V");
    chk_chinese
        .set_rect(Rect::new(50.0, 650.0, 70.0, 670.0))
        .expect("rect");
    chk_chinese
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(chk_chinese.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // Part C: Reverse order (call update BEFORE setting /AS) to demonstrate /Yes fallback
    let mut chk_uninit = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("chk_uninit");
    let mut cu_obj = chk_uninit.object();
    cu_obj
        .dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
        .expect("FT");
    cu_obj
        .dict_put("T", PdfObject::new_string("UninitCheck").expect("T"))
        .expect("T");
    // Leave /AS and /V unset
    chk_uninit
        .set_rect(Rect::new(50.0, 600.0, 70.0, 620.0))
        .expect("rect");
    chk_uninit
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(chk_uninit.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // Update all widgets
    for mut w in page.widgets() {
        let _ = w.update();
    }
    // Now reset Large's AS to Off because Small was selected initially
    rl_obj
        .dict_put("AS", PdfObject::new_name("Off").expect("Off"))
        .expect("put Off");

    // Verify Part A state names:
    let ap_agree = ca_obj.get_dict("AP").expect("get AP").expect("AP");
    let ap_n_agree = ap_agree.get_dict("N").expect("get N").expect("N");
    assert!(
        ap_n_agree.get_dict("Agree").expect("get Agree").is_some(),
        "Must have Agree state"
    );
    assert!(
        ap_n_agree.get_dict("Off").expect("get Off").is_some(),
        "Must have Off state"
    );

    let ap_rs = rs_obj.get_dict("AP").expect("get AP").expect("AP");
    let ap_n_rs = ap_rs.get_dict("N").expect("get N").expect("N");
    assert!(
        ap_n_rs.get_dict("Small").expect("get Small").is_some(),
        "Must have Small state"
    );
    assert!(
        ap_n_rs.get_dict("Off").expect("get Off").is_some(),
        "Must have Off state"
    );

    let ap_rl = rl_obj.get_dict("AP").expect("get AP").expect("AP");
    let ap_n_rl = ap_rl.get_dict("N").expect("get N").expect("N");
    assert!(
        ap_n_rl.get_dict("Large").expect("get Large").is_some(),
        "Must have Large state"
    );
    assert!(
        ap_n_rl.get_dict("Off").expect("get Off").is_some(),
        "Must have Off state"
    );

    // Verify Part B (Chinese state names):
    let ap_cc = cc_obj.get_dict("AP").expect("get AP").expect("AP");
    let ap_n_cc = ap_cc.get_dict("N").expect("get N").expect("N");
    assert!(
        ap_n_cc.get_dict("同意").expect("get 同意").is_some(),
        "Must have Chinese state /同意"
    );
    assert!(
        ap_n_cc.get_dict("Off").expect("get Off").is_some(),
        "Must have Off state"
    );

    // Verify Part C (Reverse order: MuPDF fell back to /Yes):
    let ap_cu = cu_obj.get_dict("AP").expect("get AP").expect("AP");
    let ap_n_cu = ap_cu.get_dict("N").expect("get N").expect("N");
    assert!(
        ap_n_cu.get_dict("Yes").expect("get Yes").is_some(),
        "Without setting /AS first, MuPDF falls back to /Yes per pdf_button_field_on_state (pdf-form.c:518)"
    );

    // Save and test with engine_doc
    let mut pdf_bytes = Vec::new();
    let mut write_opts = PdfWriteOptions::default();
    write_opts.set_appearance(true);
    doc.write_to_with_options(&mut pdf_bytes, write_opts)
        .expect("write_to");
    check_qpdf_bytes(&pdf_bytes);

    let mut engine_doc = PdfDocument::from_bytes(&pdf_bytes).expect("open engine doc");
    let pf = engine_doc.page_fields(0).expect("page_fields");

    let f_agree = pf
        .iter()
        .find(|f| f.label.as_deref() == Some("Terms Agreement"))
        .expect("AgreeBox");
    assert_eq!(f_agree.on_value.as_deref(), Some("Agree"));
    assert_eq!(f_agree.value, "Agree");

    let f_chinese = pf
        .iter()
        .find(|f| f.label.as_deref() == Some("同意授權"))
        .expect("ChineseAgree");
    assert_eq!(f_chinese.on_value.as_deref(), Some("同意"));
    assert_eq!(f_chinese.value, "同意");

    // Test toggling with set_field_value and save_checked
    let agree_id = f_agree.id;
    engine_doc
        .set_field_value(0, agree_id, "Off")
        .expect("set Off");
    let pf_toggled = engine_doc.page_fields(0).expect("page_fields");
    let f_agree_toggled = pf_toggled.iter().find(|f| f.id == agree_id).expect("f");
    assert_eq!(f_agree_toggled.value, "Off");

    let mut saved_bytes = Vec::new();
    engine_doc
        .save_checked(&mut saved_bytes)
        .expect("save_checked");
    check_qpdf_bytes(&saved_bytes);
    println!(
        "Item 2: Button state names, Chinese export values, and reverse ordering /Yes fallback verified."
    );
}

// ============================================================================
// Item 3: Chinese Character Support (Values, Options, Tooltips, Rendering, Flattening)
// ============================================================================
#[test]
fn test_item3_chinese_character_support() {
    let mut doc = MuPdf::new();
    let mut page = doc.new_page(Size::A4).expect("new page");
    let mut fields = get_or_create_acroform_fields(&mut doc);

    // 1. Text field with Chinese value and tooltip
    let mut text1 = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("text1");
    let mut t1_obj = text1.object();
    t1_obj
        .dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
        .expect("FT");
    t1_obj
        .dict_put("T", PdfObject::new_string("TextChineseName").expect("T"))
        .expect("T");
    t1_obj
        .dict_put("TU", PdfObject::new_string("請填寫姓名").expect("TU"))
        .expect("TU");
    t1_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    t1_obj
        .dict_put("V", PdfObject::new_string("王小明").expect("V"))
        .expect("V");
    text1
        .set_rect(Rect::new(50.0, 700.0, 250.0, 730.0))
        .expect("rect");
    text1.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
    fields
        .array_push(
            doc.new_indirect(text1.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 2. Single Chinese character fields for rendering distinctness
    let mut text_wang = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("text_wang");
    let mut tw_obj = text_wang.object();
    tw_obj
        .dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
        .expect("FT");
    tw_obj
        .dict_put("T", PdfObject::new_string("CharWang").expect("T"))
        .expect("T");
    tw_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    tw_obj
        .dict_put("V", PdfObject::new_string("王").expect("V"))
        .expect("V");
    text_wang
        .set_rect(Rect::new(50.0, 650.0, 100.0, 680.0))
        .expect("rect");
    text_wang
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(text_wang.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    let mut text_ming = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("text_ming");
    let mut tm_obj = text_ming.object();
    tm_obj
        .dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
        .expect("FT");
    tm_obj
        .dict_put("T", PdfObject::new_string("CharMing").expect("T"))
        .expect("T");
    tm_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    tm_obj
        .dict_put("V", PdfObject::new_string("明").expect("V"))
        .expect("V");
    text_ming
        .set_rect(Rect::new(120.0, 650.0, 170.0, 680.0))
        .expect("rect");
    text_ming
        .set_flags(AnnotationFlags::IS_PRINT)
        .expect("flags");
    fields
        .array_push(
            doc.new_indirect(text_ming.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 3. Dropdown with Chinese options
    let mut combo = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("combo");
    let mut combo_obj = combo.object();
    combo_obj
        .dict_put("FT", PdfObject::new_name("Ch").expect("FT"))
        .expect("FT");
    combo_obj
        .dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::COMBO.bits()).expect("Ff"),
        )
        .expect("Ff");
    combo_obj
        .dict_put("T", PdfObject::new_string("CityCombo").expect("T"))
        .expect("T");
    combo_obj
        .dict_put("TU", PdfObject::new_string("居住城市").expect("TU"))
        .expect("TU");
    combo_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    let mut c_opts = doc.new_array().expect("c_opts");
    c_opts
        .array_push(PdfObject::new_string("臺北市").expect("s"))
        .expect("push");
    c_opts
        .array_push(PdfObject::new_string("新北市").expect("s"))
        .expect("push");
    c_opts
        .array_push(PdfObject::new_string("高雄市").expect("s"))
        .expect("push");
    combo_obj.dict_put("Opt", c_opts).expect("put Opt");
    combo_obj
        .dict_put("V", PdfObject::new_string("臺北市").expect("V"))
        .expect("V");
    combo
        .set_rect(Rect::new(50.0, 600.0, 200.0, 625.0))
        .expect("rect");
    combo.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
    fields
        .array_push(
            doc.new_indirect(combo.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // 4. Listbox with Chinese options
    let mut list = page
        .create_annotation(PdfAnnotationType::Widget)
        .expect("list");
    let mut list_obj = list.object();
    list_obj
        .dict_put("FT", PdfObject::new_name("Ch").expect("FT"))
        .expect("FT");
    list_obj
        .dict_put("T", PdfObject::new_string("DistrictList").expect("T"))
        .expect("T");
    list_obj
        .dict_put("TU", PdfObject::new_string("行政區").expect("TU"))
        .expect("TU");
    list_obj
        .dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
        .expect("DA");
    let mut l_opts = doc.new_array().expect("l_opts");
    l_opts
        .array_push(PdfObject::new_string("中正區").expect("s"))
        .expect("push");
    l_opts
        .array_push(PdfObject::new_string("大安區").expect("s"))
        .expect("push");
    l_opts
        .array_push(PdfObject::new_string("信義區").expect("s"))
        .expect("push");
    list_obj.dict_put("Opt", l_opts).expect("put Opt");
    list_obj
        .dict_put("V", PdfObject::new_string("大安區").expect("V"))
        .expect("V");
    list.set_rect(Rect::new(50.0, 500.0, 200.0, 580.0))
        .expect("rect");
    list.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
    fields
        .array_push(
            doc.new_indirect(list.xref().expect("xref"), 0)
                .expect("indirect"),
        )
        .expect("push");

    // Update appearances
    for mut w in page.widgets() {
        let _ = w.update();
    }

    // Inspect the generated appearance stream font:
    let ap = t1_obj.get_dict("AP").expect("get AP").expect("AP");
    let ap_n = ap.get_dict("N").expect("get N").expect("N");
    let stream_bytes = ap_n.read_stream().expect("read stream");
    let stream_text = String::from_utf8_lossy(&stream_bytes);
    assert!(
        stream_text.contains("/Mincho") || stream_text.contains("/Ming"),
        "MuPDF appearance synthesizer must select CJK font (/Mincho or /Ming)"
    );

    let mut pdf_bytes = Vec::new();
    let mut write_opts = PdfWriteOptions::default();
    write_opts.set_appearance(true);
    doc.write_to_with_options(&mut pdf_bytes, write_opts)
        .expect("write_to");
    check_qpdf_bytes(&pdf_bytes);

    let mut engine_doc = PdfDocument::from_bytes(&pdf_bytes).expect("open engine doc");

    // Verify page_fields reads exact Chinese strings and tooltips
    let pf = engine_doc.page_fields(0).expect("page_fields");
    let f_name = pf
        .iter()
        .find(|f| f.label.as_deref() == Some("請填寫姓名"))
        .expect("f_name");
    assert_eq!(f_name.value, "王小明");

    let f_city = pf
        .iter()
        .find(|f| f.label.as_deref() == Some("居住城市"))
        .expect("f_city");
    assert_eq!(f_city.value, "臺北市");
    let city_options: Vec<_> = f_city.options.iter().map(|o| o.value.as_str()).collect();
    assert_eq!(city_options, vec!["臺北市", "新北市", "高雄市"]);

    let f_dist = pf
        .iter()
        .find(|f| f.label.as_deref() == Some("行政區"))
        .expect("f_dist");
    assert_eq!(f_dist.value, "大安區");

    // Verify render_png draws Chinese characters
    let png_bytes = engine_doc.render_png(0, 1.0).expect("render png");
    let img = Image::from_bytes(&png_bytes).expect("image");
    let pixmap = img.to_pixmap().expect("pixmap");
    let samples = pixmap.samples();
    let n = pixmap.n() as usize;
    let stride = pixmap.stride() as usize;

    let extract_sub_samples = |x0: usize, x1: usize, y0: usize, y1: usize| -> Vec<u8> {
        let mut patch = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let off = y * stride + x * n;
                if off + 2 < samples.len() {
                    patch.push(samples[off]);
                    patch.push(samples[off + 1]);
                    patch.push(samples[off + 2]);
                }
            }
        }
        patch
    };

    let patch_wang = extract_sub_samples(50, 100, 650, 680);
    let patch_ming = extract_sub_samples(120, 170, 650, 680);
    assert_ne!(
        patch_wang, patch_ming,
        "Characters 王 and 明 must produce distinct rendering glyphs"
    );

    // Verify flatten_form extracts all Chinese strings cleanly
    engine_doc.flatten_form().expect("flatten form");
    let text_layer = engine_doc.page_text(0, 10000).expect("page text");
    let all_text = text_layer
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all_text.contains("王小明"),
        "Flattened text contains 王小明"
    );
    assert!(
        all_text.contains("臺北市"),
        "Flattened text contains 臺北市"
    );
    assert!(
        all_text.contains("大安區"),
        "Flattened text contains 大安區"
    );
    println!(
        "Item 3: Chinese characters in values, options, tooltips, rendering, and flattening verified."
    );
}

// ============================================================================
// Item 4: Sample Generation and Reader Test Files
// ============================================================================

fn generate_all_samples(out_dir: &std::path::Path) {
    // 1. fields-all.pdf: Covers all field types, English and Chinese
    {
        let mut doc = MuPdf::new();
        let mut page = doc.new_page(Size::A4).expect("new page");
        let mut fields = get_or_create_acroform_fields(&mut doc);

        // Text
        let mut text1 = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut t1 = text1.object();
        t1.dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
            .expect("FT");
        t1.dict_put("T", PdfObject::new_string("FullName").expect("T"))
            .expect("T");
        t1.dict_put("TU", PdfObject::new_string("姓名 Full Name").expect("TU"))
            .expect("TU");
        t1.dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
            .expect("DA");
        t1.dict_put("V", PdfObject::new_string("王小明 John Doe").expect("V"))
            .expect("V");
        text1
            .set_rect(Rect::new(50.0, 750.0, 300.0, 780.0))
            .expect("rect");
        text1.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(text1.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        // Multiline
        let mut text2 = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut t2 = text2.object();
        t2.dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
            .expect("FT");
        t2.dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::MULTILINE.bits()).expect("Ff"),
        )
        .expect("Ff");
        t2.dict_put("T", PdfObject::new_string("Comments").expect("T"))
            .expect("T");
        t2.dict_put("DA", PdfObject::new_string("/Helv 10 Tf 0 g").expect("DA"))
            .expect("DA");
        t2.dict_put(
            "V",
            PdfObject::new_string("第一行備註\nLine 2 remarks\n第三行").expect("V"),
        )
        .expect("V");
        text2
            .set_rect(Rect::new(50.0, 680.0, 300.0, 740.0))
            .expect("rect");
        text2.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(text2.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        // Password
        let mut text3 = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut t3 = text3.object();
        t3.dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
            .expect("FT");
        t3.dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::PASSWORD.bits()).expect("Ff"),
        )
        .expect("Ff");
        t3.dict_put("T", PdfObject::new_string("Passcode").expect("T"))
            .expect("T");
        t3.dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
            .expect("DA");
        t3.dict_put("V", PdfObject::new_string("Pass123").expect("V"))
            .expect("V");
        text3
            .set_rect(Rect::new(50.0, 640.0, 200.0, 670.0))
            .expect("rect");
        text3.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(text3.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        // Checkbox
        let mut chk = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut tc = chk.object();
        tc.dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
            .expect("FT");
        tc.dict_put("T", PdfObject::new_string("Subscribe").expect("T"))
            .expect("T");
        tc.dict_put("TU", PdfObject::new_string("訂閱電子報").expect("TU"))
            .expect("TU");
        tc.dict_put("AS", PdfObject::new_name("Yes").expect("AS"))
            .expect("AS");
        tc.dict_put("V", PdfObject::new_name("Yes").expect("V"))
            .expect("V");
        chk.set_rect(Rect::new(50.0, 600.0, 70.0, 620.0))
            .expect("rect");
        chk.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(chk.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        // Radio group
        let mut rp = doc.new_dict().expect("rp");
        rp.dict_put("FT", PdfObject::new_name("Btn").expect("FT"))
            .expect("FT");
        rp.dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::RADIO.bits() | FieldFlags::NO_TOGGLE_TO_OFF.bits())
                .expect("Ff"),
        )
        .expect("Ff");
        rp.dict_put("T", PdfObject::new_string("PlanSelection").expect("T"))
            .expect("T");
        rp.dict_put("V", PdfObject::new_name("Standard").expect("V"))
            .expect("V");
        let mut rkids = doc.new_array().expect("rkids");

        let mut r1 = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("r1");
        let mut r1o = r1.object();
        r1o.dict_put("AS", PdfObject::new_name("Standard").expect("AS"))
            .expect("AS");
        r1.set_rect(Rect::new(50.0, 560.0, 70.0, 580.0))
            .expect("rect");
        r1.set_flags(AnnotationFlags::IS_PRINT).expect("flags");

        let mut r2 = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("r2");
        let mut r2o = r2.object();
        r2o.dict_put("AS", PdfObject::new_name("Pro").expect("AS"))
            .expect("AS");
        r2.set_rect(Rect::new(100.0, 560.0, 120.0, 580.0))
            .expect("rect");
        r2.set_flags(AnnotationFlags::IS_PRINT).expect("flags");

        rkids
            .array_push(
                doc.new_indirect(r1.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");
        rkids
            .array_push(
                doc.new_indirect(r2.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");
        rp.dict_put("Kids", rkids).expect("put Kids");

        let rp_obj = doc.create_object().expect("obj");
        let rp_num = rp_obj.as_indirect().expect("num");
        let mut rp_indir = doc.new_indirect(rp_num, 0).expect("indir");
        r1o.dict_put("Parent", doc.new_indirect(rp_num, 0).expect("p"))
            .expect("put");
        r2o.dict_put("Parent", doc.new_indirect(rp_num, 0).expect("p"))
            .expect("put");
        rp_indir.write_object(&rp).expect("write");
        fields.array_push(rp_indir).expect("push");

        // Combo
        let mut combo = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut co = combo.object();
        co.dict_put("FT", PdfObject::new_name("Ch").expect("FT"))
            .expect("FT");
        co.dict_put(
            "Ff",
            PdfObject::new_int(FieldFlags::COMBO.bits()).expect("Ff"),
        )
        .expect("Ff");
        co.dict_put("T", PdfObject::new_string("Country").expect("T"))
            .expect("T");
        co.dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
            .expect("DA");
        let mut copts = doc.new_array().expect("opts");
        copts
            .array_push(PdfObject::new_string("臺灣 Taiwan").expect("s"))
            .expect("push");
        copts
            .array_push(PdfObject::new_string("日本 Japan").expect("s"))
            .expect("push");
        co.dict_put("Opt", copts).expect("Opt");
        co.dict_put("V", PdfObject::new_string("臺灣 Taiwan").expect("V"))
            .expect("V");
        combo
            .set_rect(Rect::new(50.0, 510.0, 200.0, 540.0))
            .expect("rect");
        combo.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(combo.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        // List
        let mut list = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut lo = list.object();
        lo.dict_put("FT", PdfObject::new_name("Ch").expect("FT"))
            .expect("FT");
        lo.dict_put("T", PdfObject::new_string("Interest").expect("T"))
            .expect("T");
        lo.dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
            .expect("DA");
        let mut lopts = doc.new_array().expect("opts");
        lopts
            .array_push(PdfObject::new_string("閱讀 Reading").expect("s"))
            .expect("push");
        lopts
            .array_push(PdfObject::new_string("運動 Sports").expect("s"))
            .expect("push");
        lo.dict_put("Opt", lopts).expect("Opt");
        lo.dict_put("V", PdfObject::new_string("閱讀 Reading").expect("V"))
            .expect("V");
        list.set_rect(Rect::new(50.0, 420.0, 200.0, 490.0))
            .expect("rect");
        list.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(list.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        // Signature
        let sig = page.add_signature_widget("SignHere").expect("sig");
        let mut sig_annot = sig.into_annotation();
        sig_annot
            .set_rect(Rect::new(50.0, 330.0, 250.0, 400.0))
            .expect("rect");
        sig_annot
            .set_flags(AnnotationFlags::IS_PRINT)
            .expect("flags");
        let mut so = sig_annot.object();
        let _ = so.dict_delete("Lock");

        for mut w in page.widgets() {
            let _ = w.update();
        }
        r2o.dict_put("AS", PdfObject::new_name("Off").expect("Off"))
            .expect("Off");

        let mut bytes = Vec::new();
        let mut wopts = PdfWriteOptions::default();
        wopts.set_appearance(true);
        doc.write_to_with_options(&mut bytes, wopts).expect("write");
        check_qpdf_bytes(&bytes);
        fs::write(out_dir.join("fields-all.pdf"), &bytes).expect("write fields-all.pdf");
    }

    // 2. tabs-*.pdf: 5 text fields where Annots array order != visual order
    let tab_variants: Vec<(&str, Option<&str>)> = vec![
        ("tabs-none.pdf", None),
        ("tabs-r.pdf", Some("R")),
        ("tabs-c.pdf", Some("C")),
        ("tabs-s.pdf", Some("S")),
        ("tabs-a.pdf", Some("A")),
        ("tabs-w.pdf", Some("W")),
    ];

    for (filename, tab_mode) in tab_variants {
        let mut doc = MuPdf::new();
        let mut page = doc.new_page(Size::A4).expect("new page");
        let mut fields = get_or_create_acroform_fields(&mut doc);

        // Visual order top to bottom:
        // A (750), B (680), C (610), D (540), E (470)
        // Array insertion order: [C, A, E, B, D]
        let field_defs = [
            ("Field_C", "3. Third Field (Y=610)", 610.0),
            ("Field_A", "1. Top Field (Y=750)", 750.0),
            ("Field_E", "5. Bottom Field (Y=470)", 470.0),
            ("Field_B", "2. Second Field (Y=680)", 680.0),
            ("Field_D", "4. Fourth Field (Y=540)", 540.0),
        ];

        for (name, val, y) in field_defs {
            let mut tf = page
                .create_annotation(PdfAnnotationType::Widget)
                .expect("w");
            let mut obj = tf.object();
            obj.dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
                .expect("FT");
            obj.dict_put("T", PdfObject::new_string(name).expect("T"))
                .expect("T");
            obj.dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
                .expect("DA");
            obj.dict_put("V", PdfObject::new_string(val).expect("V"))
                .expect("V");
            tf.set_rect(Rect::new(50.0, y, 300.0, y + 30.0))
                .expect("rect");
            tf.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
            fields
                .array_push(
                    doc.new_indirect(tf.xref().expect("xref"), 0)
                        .expect("indir"),
                )
                .expect("push");
        }

        if let Some(mode) = tab_mode {
            let mut p_obj = page.object();
            p_obj
                .dict_put("Tabs", PdfObject::new_name(mode).expect("mode"))
                .expect("Tabs");
        }

        for mut w in page.widgets() {
            let _ = w.update();
        }

        let mut bytes = Vec::new();
        let mut wopts = PdfWriteOptions::default();
        wopts.set_appearance(true);
        doc.write_to_with_options(&mut bytes, wopts).expect("write");
        check_qpdf_bytes(&bytes);
        fs::write(out_dir.join(filename), &bytes).expect("write tab pdf");
    }

    // 3. signature-arrow.pdf: Default MuPDF unsigned sign arrow
    {
        let mut doc = MuPdf::new();
        let mut page = doc.new_page(Size::A4).expect("new page");
        let _ = get_or_create_acroform_fields(&mut doc);

        let sig = page.add_signature_widget("SignArrowField").expect("sig");
        let mut sig_annot = sig.into_annotation();
        sig_annot
            .set_rect(Rect::new(50.0, 500.0, 250.0, 560.0))
            .expect("rect");
        sig_annot
            .set_flags(AnnotationFlags::IS_PRINT)
            .expect("flags");
        let mut so = sig_annot.object();
        let _ = so.dict_delete("Lock");

        for mut w in page.widgets() {
            let _ = w.update();
        }

        let mut bytes = Vec::new();
        let mut wopts = PdfWriteOptions::default();
        wopts.set_appearance(true);
        doc.write_to_with_options(&mut bytes, wopts).expect("write");
        check_qpdf_bytes(&bytes);
        fs::write(out_dir.join("signature-arrow.pdf"), &bytes).expect("write signature-arrow.pdf");
    }

    // 4. signature-border.pdf: Border-only without arrow
    {
        let mut doc = MuPdf::new();
        let mut page = doc.new_page(Size::A4).expect("new page");
        let _ = get_or_create_acroform_fields(&mut doc);

        let sig = page.add_signature_widget("SignBorderField").expect("sig");
        let mut sig_annot = sig.into_annotation();
        sig_annot
            .set_rect(Rect::new(50.0, 500.0, 250.0, 560.0))
            .expect("rect");
        sig_annot
            .set_flags(AnnotationFlags::IS_PRINT)
            .expect("flags");
        let mut so = sig_annot.object();
        let _ = so.dict_delete("Lock");

        // Synthesize standard appearances first
        for mut w in page.widgets() {
            let _ = w.update();
        }

        // Replace /AP /N with custom border-only stream
        let border_ap =
            create_signature_border_appearance(&mut doc, 200.0, 60.0).expect("border ap");
        let mut ap_dict = doc.new_dict().expect("new dict");
        ap_dict.dict_put("N", border_ap).expect("put N");
        so.dict_put("AP", ap_dict).expect("put AP");

        let mut bytes = Vec::new();
        // Save without write_opts.set_appearance to keep our custom AP intact
        doc.write_to(&mut bytes).expect("write");
        check_qpdf_bytes(&bytes);
        fs::write(out_dir.join("signature-border.pdf"), &bytes)
            .expect("write signature-border.pdf");
    }
}

static SAMPLES_INIT: Once = Once::new();

fn ensure_all_samples(out_dir: &std::path::Path) {
    SAMPLES_INIT.call_once(|| {
        generate_all_samples(out_dir);
    });
}

#[test]
fn test_item4_sample_generation() {
    let out_dir = get_poc_output_dir();
    ensure_all_samples(&out_dir);
    println!(
        "Item 4: Generated all 9 sample PDF files to {}",
        out_dir.display()
    );
}

// ============================================================================
// Item 5: Security and Integrity Audit
// ============================================================================
#[test]
fn test_item5_security_and_integrity_audit() {
    let out_dir = get_poc_output_dir();
    ensure_all_samples(&out_dir);
    let samples = [
        "fields-all.pdf",
        "tabs-none.pdf",
        "tabs-r.pdf",
        "tabs-c.pdf",
        "tabs-s.pdf",
        "tabs-a.pdf",
        "tabs-w.pdf",
        "signature-arrow.pdf",
        "signature-border.pdf",
    ];

    // Audit each generated sample PDF
    for sample_name in samples {
        let path = out_dir.join(sample_name);
        assert!(path.exists(), "Sample file {sample_name} must exist");
        let bytes = fs::read(&path).expect("read sample");

        // 1. QPDF check
        check_qpdf_bytes(&bytes);

        // 2. MuPDF recursive security audit
        let doc = MuPdf::from_bytes(&bytes).expect("open MuPdf");
        audit_pdf_security(&doc).unwrap_or_else(|e| panic!("Audit failed on {sample_name}: {e}"));

        // 3. Check every widget has /AP /N
        let page = doc.load_pdf_page(0).expect("load page");
        for w in page.widgets() {
            let obj = w.annotation().object();
            let ap = obj
                .get_dict("AP")
                .expect("get AP")
                .expect("widget must have /AP");
            let ap_n = ap.get_dict("N").expect("get N").expect("/AP must have /N");
            assert!(
                ap_n.is_dict().unwrap_or(false) || ap_n.is_stream().unwrap_or(false),
                "/AP /N must be a stream or dictionary in {sample_name}"
            );
        }
    }

    // 4. Injection test: payload in field name and tooltip
    {
        let mut doc = MuPdf::new();
        let mut page = doc.new_page(Size::A4).expect("new page");
        let mut fields = get_or_create_acroform_fields(&mut doc);

        let injection_name = "test) /AA << /S /JavaScript /JS (app.alert(1)) >> (";
        let injection_tip = "tooltip) /A << /S /URI /URI (http://evil.com) >> (";

        let mut field = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut fo = field.object();
        fo.dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
            .expect("FT");
        fo.dict_put("T", PdfObject::new_string(injection_name).expect("T"))
            .expect("T");
        fo.dict_put("TU", PdfObject::new_string(injection_tip).expect("TU"))
            .expect("TU");
        fo.dict_put("DA", PdfObject::new_string("/Helv 12 Tf 0 g").expect("DA"))
            .expect("DA");
        fo.dict_put("V", PdfObject::new_string("SafeValue").expect("V"))
            .expect("V");
        field
            .set_rect(Rect::new(50.0, 700.0, 250.0, 730.0))
            .expect("rect");
        field.set_flags(AnnotationFlags::IS_PRINT).expect("flags");
        fields
            .array_push(
                doc.new_indirect(field.xref().expect("xref"), 0)
                    .expect("indir"),
            )
            .expect("push");

        for mut w in page.widgets() {
            let _ = w.update();
        }

        let mut bytes = Vec::new();
        let mut wopts = PdfWriteOptions::default();
        wopts.set_appearance(true);
        doc.write_to_with_options(&mut bytes, wopts).expect("write");
        check_qpdf_bytes(&bytes);

        // Verify that injection strings were properly escaped as string literals
        // and no forbidden action keys were introduced into the dictionary
        let parsed = MuPdf::from_bytes(&bytes).expect("open parsed");
        audit_pdf_security(&parsed).expect("injection test must pass security audit");

        let p_page = parsed.load_pdf_page(0).expect("page");
        let w = p_page.widgets().next().expect("w");
        assert_eq!(w.name().unwrap().as_deref(), Some(injection_name));
        assert_eq!(w.label().unwrap().as_deref(), Some(injection_tip));
    }

    // 5. Negative control: verify our security audit actually rejects forbidden keys
    {
        let mut doc = MuPdf::new();
        let mut page = doc.new_page(Size::A4).expect("new page");
        let field = page
            .create_annotation(PdfAnnotationType::Widget)
            .expect("w");
        let mut fo = field.object();
        fo.dict_put("FT", PdfObject::new_name("Tx").expect("FT"))
            .expect("FT");

        // Manually inject a forbidden /AA dictionary
        let mut aa_dict = doc.new_dict().expect("new dict");
        aa_dict
            .dict_put("E", doc.new_dict().expect("dict"))
            .expect("put E");
        fo.dict_put("AA", aa_dict).expect("put AA");

        let audit_res = audit_pdf_security(&doc);
        assert!(
            audit_res.is_err(),
            "Negative control: /AA must be caught by security audit"
        );
        let err_msg = audit_res.unwrap_err();
        assert!(
            err_msg.contains("/AA"),
            "Error message must report forbidden key: {err_msg}"
        );
    }

    println!(
        "Item 5: Recursive security audit, qpdf validation, injection robustness, and negative control verified."
    );
}
