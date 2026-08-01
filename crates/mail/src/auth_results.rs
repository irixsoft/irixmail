use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodResult {
    Pass,
    Fail,
    SoftFail,
    Neutral,
    None,
    TempError,
    PermError,
}

impl MethodResult {
    fn keyword(self) -> &'static str {
        match self {
            MethodResult::Pass => "pass",
            MethodResult::Fail => "fail",
            MethodResult::SoftFail => "softfail",
            MethodResult::Neutral => "neutral",
            MethodResult::None => "none",
            MethodResult::TempError => "temperror",
            MethodResult::PermError => "permerror",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DkimVerdict {
    pub result: MethodResult,
    pub domain: String,
    pub selector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpfVerdict {
    pub result: MethodResult,
    pub identity: SpfIdentity,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpfIdentity {
    MailFrom,
    Helo,
}

impl SpfIdentity {
    fn property(self) -> &'static str {
        match self {
            SpfIdentity::MailFrom => "smtp.mailfrom",
            SpfIdentity::Helo => "smtp.helo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmarcVerdict {
    pub result: MethodResult,
    pub from_domain: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArcVerdict {
    pub result: MethodResult,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthResults {
    pub dkim: Vec<DkimVerdict>,
    pub spf: Option<SpfVerdict>,
    pub dmarc: Option<DmarcVerdict>,
    pub arc: Option<ArcVerdict>,
}

impl AuthResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dkim(mut self, dkim: DkimVerdict) -> Self {
        self.dkim.push(dkim);
        self
    }

    pub fn with_spf(mut self, spf: SpfVerdict) -> Self {
        self.spf = Some(spf);
        self
    }

    pub fn with_dmarc(mut self, dmarc: DmarcVerdict) -> Self {
        self.dmarc = Some(dmarc);
        self
    }

    pub fn with_arc(mut self, arc: ArcVerdict) -> Self {
        self.arc = Some(arc);
        self
    }

    pub fn to_header_value(&self, authserv_id: &str) -> String {
        let mut header = String::with_capacity(64);
        header.push_str(authserv_id);

        let mut wrote_method = false;

        for dkim in &self.dkim {
            header.push_str(";\r\n\t");
            write!(
                header,
                "dkim={} header.d={}",
                dkim.result.keyword(),
                collapse_folding(&dkim.domain)
            )
            .ok();
            if let Some(selector) = &dkim.selector {
                write!(header, " header.s={}", collapse_folding(selector)).ok();
            }
            wrote_method = true;
        }

        if let Some(spf) = &self.spf {
            header.push_str(";\r\n\t");
            write!(
                header,
                "spf={} {}={}",
                spf.result.keyword(),
                spf.identity.property(),
                collapse_folding(&spf.value)
            )
            .ok();
            wrote_method = true;
        }

        if let Some(dmarc) = &self.dmarc {
            header.push_str(";\r\n\t");
            write!(
                header,
                "dmarc={} header.from={}",
                dmarc.result.keyword(),
                collapse_folding(&dmarc.from_domain)
            )
            .ok();
            wrote_method = true;
        }

        if let Some(arc) = &self.arc {
            header.push_str(";\r\n\t");
            write!(header, "arc={}", arc.result.keyword()).ok();
            wrote_method = true;
        }

        if !wrote_method {
            header.push_str("; none");
        }

        header
    }
}

fn collapse_folding(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dkim(result: MethodResult, domain: &str, selector: Option<&str>) -> DkimVerdict {
        DkimVerdict {
            result,
            domain: domain.to_string(),
            selector: selector.map(str::to_string),
        }
    }

    #[test]
    fn empty_results_emit_the_none_token() {
        let header = AuthResults::new().to_header_value("mx.irixmail.test");
        assert_eq!(header, "mx.irixmail.test; none");
    }

    #[test]
    fn a_passing_dkim_signature_names_domain_and_selector() {
        let header = AuthResults::new()
            .with_dkim(dkim(MethodResult::Pass, "example.org", Some("sel1")))
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            "mx.irixmail.test;\r\n\tdkim=pass header.d=example.org header.s=sel1"
        );
    }

    #[test]
    fn a_dkim_signature_without_a_selector_omits_the_selector_property() {
        let header = AuthResults::new()
            .with_dkim(dkim(MethodResult::Fail, "example.org", None))
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            "mx.irixmail.test;\r\n\tdkim=fail header.d=example.org"
        );
    }

    #[test]
    fn each_evaluated_signature_contributes_its_own_dkim_clause() {
        let header = AuthResults::new()
            .with_dkim(dkim(MethodResult::Pass, "example.org", Some("a")))
            .with_dkim(dkim(MethodResult::Neutral, "relay.example", Some("b")))
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            concat!(
                "mx.irixmail.test;\r\n\tdkim=pass header.d=example.org header.s=a",
                ";\r\n\tdkim=neutral header.d=relay.example header.s=b"
            )
        );
    }

    #[test]
    fn spf_against_mail_from_uses_the_smtp_mailfrom_property() {
        let header = AuthResults::new()
            .with_spf(SpfVerdict {
                result: MethodResult::Pass,
                identity: SpfIdentity::MailFrom,
                value: "alice@example.org".to_string(),
            })
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            "mx.irixmail.test;\r\n\tspf=pass smtp.mailfrom=alice@example.org"
        );
    }

    #[test]
    fn spf_against_helo_uses_the_smtp_helo_property() {
        let header = AuthResults::new()
            .with_spf(SpfVerdict {
                result: MethodResult::SoftFail,
                identity: SpfIdentity::Helo,
                value: "mail.example.org".to_string(),
            })
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            "mx.irixmail.test;\r\n\tspf=softfail smtp.helo=mail.example.org"
        );
    }

    #[test]
    fn dmarc_reports_against_the_author_domain() {
        let header = AuthResults::new()
            .with_dmarc(DmarcVerdict {
                result: MethodResult::Pass,
                from_domain: "example.org".to_string(),
            })
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            "mx.irixmail.test;\r\n\tdmarc=pass header.from=example.org"
        );
    }

    #[test]
    fn arc_reports_only_the_chain_result() {
        let header = AuthResults::new()
            .with_arc(ArcVerdict {
                result: MethodResult::Pass,
            })
            .to_header_value("mx.irixmail.test");
        assert_eq!(header, "mx.irixmail.test;\r\n\tarc=pass");
    }

    #[test]
    fn methods_are_emitted_in_a_stable_order_with_semicolon_separators() {
        let header = AuthResults::new()
            .with_dkim(dkim(MethodResult::Pass, "example.org", Some("sel")))
            .with_spf(SpfVerdict {
                result: MethodResult::Pass,
                identity: SpfIdentity::MailFrom,
                value: "alice@example.org".to_string(),
            })
            .with_dmarc(DmarcVerdict {
                result: MethodResult::Pass,
                from_domain: "example.org".to_string(),
            })
            .with_arc(ArcVerdict {
                result: MethodResult::TempError,
            })
            .to_header_value("mx.irixmail.test");
        assert_eq!(
            header,
            concat!(
                "mx.irixmail.test",
                ";\r\n\tdkim=pass header.d=example.org header.s=sel",
                ";\r\n\tspf=pass smtp.mailfrom=alice@example.org",
                ";\r\n\tdmarc=pass header.from=example.org",
                ";\r\n\tarc=temperror"
            )
        );
    }

    #[test]
    fn a_value_carrying_cr_lf_cannot_inject_extra_header_lines() {
        let header = AuthResults::new()
            .with_dkim(dkim(
                MethodResult::Pass,
                "evil.org\r\nInjected: yes",
                Some("a\r\nb"),
            ))
            .with_spf(SpfVerdict {
                result: MethodResult::Pass,
                identity: SpfIdentity::MailFrom,
                value: "alice@evil.org\r\nX-Spoof: 1".to_string(),
            })
            .with_dmarc(DmarcVerdict {
                result: MethodResult::Pass,
                from_domain: "evil.org\nLeak: 1".to_string(),
            })
            .to_header_value("mx.irixmail.test");
        assert!(!header.contains("\r\nInjected:"));
        assert!(!header.contains("\r\nX-Spoof:"));
        assert!(!header.contains("\nLeak:"));
        assert!(header.contains("header.d=evil.org  Injected: yes"));
        assert!(header.contains("header.s=a  b"));
        assert!(header.contains("smtp.mailfrom=alice@evil.org  X-Spoof: 1"));
        assert!(header.contains("header.from=evil.org Leak: 1"));
    }

    #[test]
    fn every_result_keyword_matches_the_rfc_vocabulary() {
        assert_eq!(MethodResult::Pass.keyword(), "pass");
        assert_eq!(MethodResult::Fail.keyword(), "fail");
        assert_eq!(MethodResult::SoftFail.keyword(), "softfail");
        assert_eq!(MethodResult::Neutral.keyword(), "neutral");
        assert_eq!(MethodResult::None.keyword(), "none");
        assert_eq!(MethodResult::TempError.keyword(), "temperror");
        assert_eq!(MethodResult::PermError.keyword(), "permerror");
    }
}
