// Replaces the generated `coriolis-data/dist` bundle with the raw upstream JSON sources.
module.exports = {
  Ships: require('../../upstream/coriolis-data/ships'),
  Modules: require('../../upstream/coriolis-data/modules'),
  Modifications: require('../../upstream/coriolis-data/modifications'),
};
