#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) enum MatchType {
    Is,
    Contains,
    Matches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) enum Comparator {
    AsciiCaseMap,
    Octet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) enum AddressPart {
    All,
    Localpart,
    Domain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) enum EnvelopePart {
    From,
    To,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) enum Test {
    Header {
        headers: Vec<String>,
        keys: Vec<String>,
        match_type: MatchType,
        comparator: Comparator,
        is_not: bool,
    },
    Address {
        headers: Vec<String>,
        keys: Vec<String>,
        part: AddressPart,
        match_type: MatchType,
        comparator: Comparator,
        is_not: bool,
    },
    Envelope {
        parts: Vec<EnvelopePart>,
        keys: Vec<String>,
        part: AddressPart,
        match_type: MatchType,
        comparator: Comparator,
        is_not: bool,
    },
    Exists {
        headers: Vec<String>,
        is_not: bool,
    },
    Size {
        over: bool,
        limit: u64,
        is_not: bool,
    },
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) enum Instruction {
    Test(Test),
    Jmp(usize),
    Jz(usize),
    Jnz(usize),
    Keep,
    Discard,
    Stop,
    FileInto(String),
    Redirect(String),
    AddFlag(Vec<String>),
    SetFlag(Vec<String>),
    RemoveFlag(Vec<String>),
}
