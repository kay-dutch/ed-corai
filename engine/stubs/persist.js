// Persist stand-in: no localStorage in Node; return defaults.
const Persist = {
  getModuleDiscount: () => 0,
  getShipDiscount: () => 0,
  getInsurance: () => 'standard',
  getCostTab: () => 'costs',
  getOutfittingTab: () => null,
  setOutfittingTab: () => {},
  addListener: () => ({ remove() {} }),
};
export default Persist;
