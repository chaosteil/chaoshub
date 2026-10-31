//! Maps the merge settings of the configuration to the domain, through
//! the conversion in `config`.

use crate::config::schema::MergeMethodSetting;
use crate::domain::merge::MergeMethod;

/// The domain merge method of a `[merge.repos."owner/name"] method`.
#[allow(
    dead_code,
    reason = "the merge action of phase E resolves the method of a repo with it"
)]
pub(crate) fn merge_method(setting: MergeMethodSetting) -> MergeMethod {
    MergeMethod::from(setting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_maps() {
        let cases = [
            (MergeMethodSetting::Merge, MergeMethod::Merge),
            (MergeMethodSetting::Squash, MergeMethod::Squash),
            (MergeMethodSetting::Rebase, MergeMethod::Rebase),
        ];
        for (setting, method) in cases {
            assert_eq!(merge_method(setting), method);
        }
    }
}
