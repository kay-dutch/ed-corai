// Replaces the generated `coriolis-data/dist` bundle with the raw upstream JSON sources.
const Ships = require('../../upstream/coriolis-data/ships');

// Coriolis has no heat dissipation figure for the Anaconda Mk II yet; until it does, ED Corai
// assumes the old Anaconda's (the project's rule for unknown Mk II values). corai-data's build.rs
// applies the same assumption.
const mk2 = Ships.anaconda2 && Ships.anaconda2.properties;
if (mk2 && mk2.heatDissipation == null) mk2.heatDissipation = Ships.anaconda.properties.heatDissipation;

module.exports = {
  Ships,
  Modules: require('../../upstream/coriolis-data/modules'),
  Modifications: require('../../upstream/coriolis-data/modifications'),
  // Authoritative journal slot names per ship (generate_distribution.js ships it as ShipSlots).
  ShipSlots: require('../../upstream/coriolis-data/ships/ship-slots.json'),
};
