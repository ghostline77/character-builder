use crate::{catalogue::Catalogue, project::Project};
use anyhow::{Context, Result};

/// Builds the character, AI and equipment record graph consumed by TweakXL.
pub fn render(project: &Project, catalogue: &Catalogue) -> Result<String> {
    let combat = !project.loadout.is_empty()
        || project.ai.profile.starts_with("native:")
        || matches!(
            project.ai.profile.as_str(),
            "combat" | "cyberpsycho_netrunner"
        );
    let base = if combat {
        "Character.Quest_Combat_NPC_Base"
    } else {
        "Character.Quest_NPC_Base"
    };
    let mut tweak = format!(
        "{}:\n  $base: {base}\n  entityTemplatePath: {}\n",
        project.identity.record,
        serde_json::to_string(&format!("{}\\{}.ent", project.namespace, project.id))?
    );
    if !project.identity.display_name.is_empty() {
        let name = serde_json::to_string(&project.identity.display_name)?;
        tweak.push_str(&format!(
            "  displayName: {name}\n  fullDisplayName: {name}\n"
        ));
    }
    if let Some(preset) = crate::ai::preset(&project.ai.profile)? {
        for (key, value) in preset["values"]
            .as_object()
            .context("native behaviour values missing")?
        {
            tweak.push_str(&format!("  {key}: {}\n", serde_json::to_string(value)?));
        }
    }
    if project.ai.profile == "cyberpsycho_netrunner" {
        tweak.push_str("  actionMap: MaxTacNetrunner.Map\n  archetypeData: ArchetypeData.NetrunnerT3\n  baseAttitudeGroup: neutral\n  rarity: NPCRarity.Boss\n  reactionPreset: ReactionPresets.Ganger_Aggressive\n  scannerModulePreset: ScanningNPCPresets.ScannerPreset_NPCFull\n  threatTrackingPreset: TargetTracking.DefaultPreset\n  uiNameplate: UINameplate.CombatSettings\n  disableDefeatedState: false\n  dropsWeaponOnDeath: false\n  skipDisplayArchetype: true\n  abilities:\n");
        for ability in [
            "CanCrouch",
            "CanSprint",
            "CanSwitchWeapon",
            "CanShootWhileMoving",
            "CanPreciseShoot",
            "CanChargedShoot",
            "CanSmartShoot",
            "CanUseLeftHand",
            "CanUseRightHand",
            "CanUseLegs",
            "CanTaunt",
            "CanCloseCombat",
            "HasDodge",
            "HasMemoryWipeImmunity",
        ] {
            tweak.push_str(&format!("    - Ability.{ability}\n"));
        }
        tweak.push_str("  effectors:\n    - Effectors.HitReactionTBHIncrease\n    - Character.AllowTechWeaponDodgeEffector\n  tags:\n    - Cyberpsycho\n  visualTags:\n    - Netrunner\n    - Lvl3\n");
    }
    for (key, value) in [
        ("actionMap", &project.ai.action_map),
        ("archetypeData", &project.ai.archetype_data),
        ("baseAttitudeGroup", &project.ai.attitude_group),
        ("reactionPreset", &project.ai.reaction_preset),
        ("rarity", &project.ai.rarity),
    ] {
        if !value.is_empty() {
            // Overrides replace preset fields instead of emitting duplicate YAML keys.
            let prefix = format!("  {key}:");
            tweak = tweak
                .lines()
                .filter(|line| !line.starts_with(&prefix))
                .map(|line| format!("{line}\n"))
                .collect();
            tweak.push_str(&format!("  {key}: {}\n", serde_json::to_string(value)?));
        }
    }
    let stat_group = format!("{}_Stats", project.identity.record);
    if project.ai.profile == "cyberpsycho_netrunner" || project.ai.health_multiplier.is_some() {
        tweak.push_str("  statModifierGroups:\n    - Character.NPC_Base_Primary_Stat_ModGroup\n");
        if project.ai.profile == "cyberpsycho_netrunner" {
            tweak.push_str("    - Character.Cyberpsycho_ModGroup\n    - Character.Cyberpsycho_HitReaction_Resistance\n");
        }
        if project.ai.health_multiplier.is_some() {
            tweak.push_str(&format!("    - {stat_group}\n"));
        }
    }
    if project.ai.power_level.is_some() {
        tweak.push_str(&format!(
            "  contentAssignment: {}_LevelContent\n",
            project.identity.record
        ));
    }
    for (name, value) in [
        ("voiceTag", &project.identity.voice_tag),
        ("affiliation", &project.identity.affiliation),
    ] {
        if !value.is_empty() {
            tweak.push_str(&format!("  {name}: {}\n", serde_json::to_string(value)?));
        }
    }
    if !project.loadout.is_empty() {
        let group = format!("{}_PrimaryEquipment", project.identity.record);
        tweak.push_str(&format!("  primaryEquipment: {group}\n\n{group}:\n  $type: gamedataNPCEquipmentGroup_Record\n  equipmentItems:\n"));
        for index in 0..project.loadout.len() {
            tweak.push_str(&format!("    - {group}_{index}\n"));
        }
        for (index, id) in project.loadout.iter().enumerate() {
            let entry = catalogue
                .entries
                .iter()
                .find(|e| e["kind"] == "loadout" && e["id"] == *id)
                .context("loadout record absent")?;
            let record = entry["record"]
                .as_str()
                .context("loadout record name absent")?;
            tweak.push_str(&format!("\n{group}_{index}:\n  $type: gamedataNPCEquipmentItem_Record\n  item: {}\n  equipSlot: AttachmentSlots.WeaponRight\n", serde_json::to_string(record)?));
            let (slot, condition) = match entry["role"].as_str() {
                Some("melee") => (
                    if entry["category"] == "Wea_Katana" {
                        "ItemSlotKatana"
                    } else {
                        "ItemSlotGenericMelee"
                    },
                    "BaseMeleePrimaryWeapon",
                ),
                Some("sniper") => ("ItemSlotGenericRanged", "SniperPrimaryWeaponSniper"),
                _ => ("ItemSlotGenericRanged", "SniperPrimaryWeaponRanged"),
            };
            tweak.push_str(&format!("  onBodySlot: AttachmentSlots.{slot}\n  equipCondition:\n    - WeaponConditions.{condition}EquipCondition\n  unequipCondition:\n    - WeaponConditions.{condition}UnequipCondition\n"));
        }
    }
    if let Some(health) = project.ai.health_multiplier {
        tweak.push_str(&format!("\n{stat_group}:\n  $type: gamedataStatModifierGroup_Record\n  statModifiers:\n    - {stat_group}_Health\n\n{stat_group}_Health:\n  $type: gamedataConstantStatModifier_Record\n  statType: BaseStats.Health\n  modifierType: Multiplier\n  value: {health}\n"));
    }
    if let Some(level) = project.ai.power_level {
        let record = &project.identity.record;
        tweak.push_str(&format!("\n{record}_LevelContent:\n  $type: gamedataDeviceContentAssignment_Record\n  powerLevelMod: {record}_Level\n\n{record}_Level:\n  $type: gamedataConstantStatModifier_Record\n  statType: BaseStats.PowerLevel\n  modifierType: Additive\n  value: {level}\n"));
    }
    Ok(tweak)
}
