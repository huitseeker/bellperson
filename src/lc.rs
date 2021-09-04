use std::ops::{Add, Sub};

use ff::{Field, ScalarEngine};

use crate::multiexp::DensityTracker;

/// Represents a variable in our constraint system.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Variable(pub(crate) Index);

impl Variable {
    /// This constructs a variable with an arbitrary index.
    /// Circuit implementations are not recommended to use this.
    pub fn new_unchecked(idx: Index) -> Variable {
        Variable(idx)
    }

    /// This returns the index underlying the variable.
    /// Circuit implementations are not recommended to use this.
    pub fn get_unchecked(&self) -> Index {
        self.0
    }
}

/// Represents the index of either an input variable or
/// auxiliary variable.
#[derive(Copy, Clone, PartialEq, Debug, Eq, Hash)]
pub enum Index {
    Input(usize),
    Aux(usize),
}

/// This represents a linear combination of some variables, with coefficients
/// in the scalar field of a pairing-friendly elliptic curve group.
#[derive(Clone)]
pub struct LinearCombination<E: ScalarEngine> {
    inputs: Indexer<E::Fr>,
    aux: Indexer<E::Fr>,
}

#[derive(Clone)]
struct Indexer<T> {
    values: Vec<(usize, T)>,
    // (index, key) of the last insertion operation
    last_inserted: Option<(usize, usize)>,
}

impl<T> Default for Indexer<T> {
    fn default() -> Self {
        Indexer {
            values: Vec::new(),
            last_inserted: None,
        }
    }
}

impl<T> Indexer<T> {
    pub fn new() -> Self {
        Indexer::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Indexer {
            values: Vec::with_capacity(cap),
            last_inserted: None,
        }
    }

    pub fn from_value(index: usize, value: T) -> Self {
        Indexer {
            values: vec![(index, value)],
            last_inserted: Some((0, index)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&usize, &T)> + '_ {
        self.values.iter().map(|(key, value)| (key, value))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&mut usize, &mut T)> + '_ {
        self.values.iter_mut().map(|(key, value)| (key, value))
    }

    pub fn insert_or_update<F, G>(&mut self, key: usize, insert: F, update: G)
    where
        F: FnOnce() -> T,
        G: FnOnce(&mut T),
    {
        if let Some((last_index, last_key)) = self.last_inserted {
            if last_key == key {
                // update the same key again
                update(&mut self.values[last_index].1);
                return;
            } else if last_key + 1 == key {
                // optimization for follow on updates
                let i = last_index + 1;
                if i >= self.values.len() {
                    // insert at the end
                    self.values.push((key, insert()));
                    self.last_inserted = Some((i, key));
                } else if self.values[i].0 == key {
                    // update
                    update(&mut self.values[i].1);
                } else {
                    // insert
                    self.values.insert(i, (key, insert()));
                    self.last_inserted = Some((i, key));
                }
                return;
            }
        }
        match self.values.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(i) => {
                update(&mut self.values[i].1);
            }
            Err(i) => {
                self.values.insert(i, (key, insert()));
                self.last_inserted = Some((i, key));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl<E: ScalarEngine> Default for LinearCombination<E> {
    fn default() -> Self {
        Self::zero()
    }
}

impl<E: ScalarEngine> LinearCombination<E> {
    pub fn zero() -> LinearCombination<E> {
        LinearCombination {
            inputs: Default::default(),
            aux: Default::default(),
        }
    }

    pub fn from_coeff(var: Variable, coeff: E::Fr) -> Self {
        match var {
            Variable(Index::Input(i)) => Self {
                inputs: Indexer::from_value(i, coeff),
                aux: Default::default(),
            },
            Variable(Index::Aux(i)) => Self {
                inputs: Default::default(),
                aux: Indexer::from_value(i, coeff),
            },
        }
    }

    pub fn from_variable(var: Variable) -> Self {
        Self::from_coeff(var, E::Fr::one())
    }

    pub fn iter(&self) -> impl Iterator<Item = (Variable, &E::Fr)> + '_ {
        self.inputs
            .iter()
            .map(|(k, v)| (Variable(Index::Input(*k)), v))
            .chain(self.aux.iter().map(|(k, v)| (Variable(Index::Aux(*k)), v)))
    }

    #[inline]
    pub(crate) fn iter_inputs(&self) -> impl Iterator<Item = (&usize, &E::Fr)> + '_ {
        self.inputs.iter()
    }

    #[inline]
    pub(crate) fn iter_aux(&self) -> impl Iterator<Item = (&usize, &E::Fr)> + '_ {
        self.aux.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Variable, &mut E::Fr)> + '_ {
        self.inputs
            .iter_mut()
            .map(|(k, v)| (Variable(Index::Input(*k)), v))
            .chain(
                self.aux
                    .iter_mut()
                    .map(|(k, v)| (Variable(Index::Aux(*k)), v)),
            )
    }

    #[inline]
    fn add_assign_unsimplified_input(&mut self, new_var: usize, coeff: E::Fr) {
        self.inputs
            .insert_or_update(new_var, || coeff, |val| val.add_assign(&coeff));
    }

    #[inline]
    fn add_assign_unsimplified_aux(&mut self, new_var: usize, coeff: E::Fr) {
        self.aux
            .insert_or_update(new_var, || coeff, |val| val.add_assign(&coeff));
    }

    pub fn add_unsimplified(mut self, (coeff, var): (E::Fr, Variable)) -> LinearCombination<E> {
        match var.0 {
            Index::Input(new_var) => {
                self.add_assign_unsimplified_input(new_var, coeff);
            }
            Index::Aux(new_var) => {
                self.add_assign_unsimplified_aux(new_var, coeff);
            }
        }

        self
    }

    #[inline]
    fn sub_assign_unsimplified_input(&mut self, new_var: usize, mut coeff: E::Fr) {
        coeff.negate();
        self.add_assign_unsimplified_input(new_var, coeff);
    }

    #[inline]
    fn sub_assign_unsimplified_aux(&mut self, new_var: usize, mut coeff: E::Fr) {
        coeff.negate();
        self.add_assign_unsimplified_aux(new_var, coeff);
    }

    pub fn sub_unsimplified(mut self, (coeff, var): (E::Fr, Variable)) -> LinearCombination<E> {
        match var.0 {
            Index::Input(new_var) => {
                self.sub_assign_unsimplified_input(new_var, coeff);
            }
            Index::Aux(new_var) => {
                self.sub_assign_unsimplified_aux(new_var, coeff);
            }
        }

        self
    }

    pub fn len(&self) -> usize {
        self.inputs.len() + self.aux.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty() && self.aux.is_empty()
    }

    pub(crate) fn eval(
        &self,
        mut input_density: Option<&mut DensityTracker>,
        mut aux_density: Option<&mut DensityTracker>,
        input_assignment: &[E::Fr],
        aux_assignment: &[E::Fr],
    ) -> E::Fr {
        let mut acc = E::Fr::zero();

        let one = E::Fr::one();

        for (index, coeff) in self.iter_inputs() {
            let mut tmp = input_assignment[*index];
            if coeff == &one {
                acc.add_assign(&tmp)
            } else {
                tmp.mul_assign(&coeff);
                acc.add_assign(&tmp);
            }

            if let Some(ref mut v) = input_density {
                v.inc(*index);
            }
        }

        for (index, coeff) in self.iter_aux() {
            let mut tmp = aux_assignment[*index];
            if coeff == &one {
                acc.add_assign(&tmp)
            } else {
                tmp.mul_assign(&coeff);
                acc.add_assign(&tmp);
            }

            if let Some(ref mut v) = aux_density {
                v.inc(*index);
            }
        }

        acc
    }
}

impl<E: ScalarEngine> Add<(E::Fr, Variable)> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn add(self, (coeff, var): (E::Fr, Variable)) -> LinearCombination<E> {
        self.add_unsimplified((coeff, var))
    }
}

impl<E: ScalarEngine> Sub<(E::Fr, Variable)> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    #[allow(clippy::suspicious_arithmetic_impl)]
    fn sub(self, (coeff, var): (E::Fr, Variable)) -> LinearCombination<E> {
        self.sub_unsimplified((coeff, var))
    }
}

impl<E: ScalarEngine> Add<Variable> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn add(self, other: Variable) -> LinearCombination<E> {
        self + (E::Fr::one(), other)
    }
}

impl<E: ScalarEngine> Sub<Variable> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn sub(self, other: Variable) -> LinearCombination<E> {
        self - (E::Fr::one(), other)
    }
}

impl<'a, E: ScalarEngine> Add<&'a LinearCombination<E>> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn add(mut self, other: &'a LinearCombination<E>) -> LinearCombination<E> {
        for (var, val) in other.inputs.iter() {
            self.add_assign_unsimplified_input(*var, *val);
        }

        for (var, val) in other.aux.iter() {
            self.add_assign_unsimplified_aux(*var, *val);
        }

        self
    }
}

impl<'a, E: ScalarEngine> Sub<&'a LinearCombination<E>> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn sub(mut self, other: &'a LinearCombination<E>) -> LinearCombination<E> {
        for (var, val) in other.inputs.iter() {
            self.sub_assign_unsimplified_input(*var, *val);
        }

        for (var, val) in other.aux.iter() {
            self.sub_assign_unsimplified_aux(*var, *val);
        }

        self
    }
}

impl<'a, E: ScalarEngine> Add<(E::Fr, &'a LinearCombination<E>)> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn add(mut self, (coeff, other): (E::Fr, &'a LinearCombination<E>)) -> LinearCombination<E> {
        for (var, val) in other.inputs.iter() {
            let mut tmp = *val;
            tmp.mul_assign(&coeff);
            self.add_assign_unsimplified_input(*var, tmp);
        }

        for (var, val) in other.aux.iter() {
            let mut tmp = *val;
            tmp.mul_assign(&coeff);
            self.add_assign_unsimplified_aux(*var, tmp);
        }

        self
    }
}

impl<'a, E: ScalarEngine> Sub<(E::Fr, &'a LinearCombination<E>)> for LinearCombination<E> {
    type Output = LinearCombination<E>;

    fn sub(mut self, (coeff, other): (E::Fr, &'a LinearCombination<E>)) -> LinearCombination<E> {
        for (var, val) in other.inputs.iter() {
            let mut tmp = *val;
            tmp.mul_assign(&coeff);
            self.sub_assign_unsimplified_input(*var, tmp);
        }

        for (var, val) in other.aux.iter() {
            let mut tmp = *val;
            tmp.mul_assign(&coeff);
            self.sub_assign_unsimplified_aux(*var, tmp);
        }

        self
    }
}

#[cfg(all(test, feature = "groth16"))]
mod tests {
    use super::*;
    use crate::bls::Bls12;

    #[test]
    fn test_add_simplify() {
        let n = 5;

        let mut lc = LinearCombination::<Bls12>::zero();

        let mut expected_sums = vec![<Bls12 as ScalarEngine>::Fr::zero(); n];
        let mut total_additions = 0;
        for (i, expected_sum) in expected_sums.iter_mut().enumerate() {
            for _ in 0..i + 1 {
                let coeff = <Bls12 as ScalarEngine>::Fr::one();
                lc = lc + (coeff, Variable::new_unchecked(Index::Aux(i)));
                expected_sum.add_assign(&coeff);
                total_additions += 1;
            }
        }

        // There are only as many terms as distinct variable Indexes — not one per addition operation.
        assert_eq!(n, lc.len());
        assert!(lc.len() != total_additions);

        // Each variable has the expected coefficient, the sume of those added by its Index.
        lc.iter().for_each(|(var, coeff)| match var.0 {
            Index::Aux(i) => assert_eq!(expected_sums[i], *coeff),
            _ => panic!("unexpected variable type"),
        });
    }

    #[test]
    fn test_insert_or_update() {
        let mut indexer = Indexer::new();
        let one = <Bls12 as ScalarEngine>::Fr::one();
        let mut two = one;
        two.add_assign(&one);

        indexer.insert_or_update(2, || one, |v| v.add_assign(&one));
        assert_eq!(&indexer.values, &[(2, one)]);
        assert_eq!(&indexer.last_inserted, &Some((0, 2)));

        indexer.insert_or_update(3, || one, |v| v.add_assign(&one));
        assert_eq!(&indexer.values, &[(2, one), (3, one)]);
        assert_eq!(&indexer.last_inserted, &Some((1, 3)));

        indexer.insert_or_update(1, || one, |v| v.add_assign(&one));
        assert_eq!(&indexer.values, &[(1, one), (2, one), (3, one)]);
        assert_eq!(&indexer.last_inserted, &Some((0, 1)));

        indexer.insert_or_update(2, || one, |v| v.add_assign(&one));
        assert_eq!(&indexer.values, &[(1, one), (2, two), (3, one)]);
        assert_eq!(&indexer.last_inserted, &Some((0, 1)));
    }
}