//! About dialog.

use adw::prelude::*;

use crate::paths::APP_ID;

pub fn show() {
    let about = adw::AboutDialog::builder()
        .application_name("Writing Tools")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Jesai and contributors")
        .comments(
            "Writing Tools is a free & lightweight tool that helps you improve your writing with AI, similar to Apple's Apple Intelligence feature. It works with an extensive range of AI LLMs, both online and locally run.\n\nCreated with care by Jesai, a high school student.",
        )
        .website("https://github.com/higajooj/WritingToolsLinux")
        .issue_url("https://github.com/higajooj/WritingToolsLinux/issues")
        .license_type(gtk::License::Gpl30)
        .build();
    about.add_link("Original Writing Tools", "https://github.com/TheJayTea/WritingTools");
    about.add_link("Bliss AI (Jesai's AI tutor)", "https://play.google.com/store/apps/details?id=com.jesai.blissai");
    about.add_credit_section(
        Some("Contributors"),
        &[
            "momokrono https://github.com/momokrono",
            "Cameron Redmore https://github.com/CameronRedmore",
            "Soszust40 https://github.com/Soszust40",
            "Alok Saboo https://github.com/arsaboo",
            "raghavdhingra24 https://github.com/raghavdhingra24",
            "ErrorCatDev https://github.com/ErrorCatDev",
            "Vadim Karpenko https://github.com/Vadim-Karpenko",
        ],
    );
    about.add_credit_section(
        Some("Historical contributors"),
        &[
            "Arya Mirsepasi https://github.com/Aryamirsepasi",
            "Joaov41 https://github.com/Joaov41",
            "drankush https://github.com/drankush",
            "gdmka https://github.com/gdmka",
        ],
    );
    about.add_legal_section(
        "Icons",
        None,
        gtk::License::Custom,
        Some(
            "The action icons come from Lucide (ISC, partly derived from Feather under MIT) and the provider brand marks from Simple Icons (CC0). See assets/icons/NOTICE for the full texts and the trademark note on the provider marks.",
        ),
    );
    about.present(None::<&gtk::Widget>);
}
