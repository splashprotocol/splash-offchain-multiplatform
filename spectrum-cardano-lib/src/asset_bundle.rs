use cml_chain::assets::{AssetBundle, AssetName};
use cml_chain::PolicyId;

pub trait SafeAssetBundleOps<T> {
    fn safe_set(&mut self, policy_id: PolicyId, asset_name: AssetName, value: T);
}

impl<T> SafeAssetBundleOps<T> for AssetBundle<T>
where
    T: num::CheckedAdd + num::CheckedSub + num::Zero + num::Bounded + Copy,
{
    fn safe_set(&mut self, policy_id: PolicyId, asset_name: AssetName, value: T) {
        if value.is_zero() {
            self.get_mut(&policy_id.clone())
                .and_then(|assets| assets.remove(&asset_name));
        } else {
            self.set(policy_id, asset_name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::asset_bundle::SafeAssetBundleOps;
    use crate::AssetName;
    use cml_chain::assets::{AssetBundle, AssetName as CMLAssetName};
    use cml_chain::PolicyId;

    const TEST_ASSET_POLICY: &str = "5ac3d4bdca238105a040a565e5d7e734b7c9e1630aec7650e809e34a";
    const TEST_ASSET_NAME_1: &str = "535155495254";
    const TEST_ASSET_NAME_2: &str = "535155495255";

    #[test]
    fn safe_set_should_remove_asset() {
        let policy: PolicyId = PolicyId::from_hex(TEST_ASSET_POLICY).unwrap();
        let tn_1: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_1).unwrap().into();
        let tn_2: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_2).unwrap().into();

        let mut asset1 = AssetBundle::new();
        asset1.set(policy, tn_1, 100);
        asset1.set(policy, tn_2.clone(), 200);

        asset1.safe_set(policy, tn_2.clone(), 0);

        assert_eq!(asset1.get(&policy, &tn_2.clone()), None)
    }

    #[test]
    fn safe_set_should_correctly_set_new_asset_value() {
        let policy: PolicyId = PolicyId::from_hex(TEST_ASSET_POLICY).unwrap();
        let tn_1: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_1).unwrap().into();
        let tn_2: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_2).unwrap().into();

        let mut asset1 = AssetBundle::new();
        asset1.set(policy, tn_1, 100);
        asset1.set(policy, tn_2.clone(), 100);

        asset1.safe_set(policy, tn_2.clone(), 150);

        assert_eq!(asset1.get(&policy, &tn_2.clone()), Some(150))
    }

    #[test]
    fn safe_set_should_correctly_insert_zero_value() {
        let policy: PolicyId = PolicyId::from_hex(TEST_ASSET_POLICY).unwrap();
        let tn_1: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_1).unwrap().into();
        let tn_2: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_2).unwrap().into();

        let mut asset1 = AssetBundle::new();
        asset1.set(policy, tn_1, 100);

        asset1.safe_set(policy, tn_2.clone(), 0);

        assert_eq!(asset1.get(&policy, &tn_2.clone()), None)
    }

    #[test]
    fn safe_set_should_correctly_insert_non_zero_value() {
        let policy: PolicyId = PolicyId::from_hex(TEST_ASSET_POLICY).unwrap();
        let tn_1: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_1).unwrap().into();
        let tn_2: CMLAssetName = AssetName::try_from_hex(TEST_ASSET_NAME_2).unwrap().into();

        let mut asset1 = AssetBundle::new();
        asset1.set(policy, tn_1, 100);

        asset1.safe_set(policy, tn_2.clone(), 100);

        assert_eq!(asset1.get(&policy, &tn_2.clone()), Some(100))
    }
}
