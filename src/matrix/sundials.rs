use std::{
    cmp::min,
    fmt::Display,
    ops::{Add, AddAssign, Div, DivAssign, Index, IndexMut, Mul, MulAssign, Sub, SubAssign},
};

use sundials_sys::{
    realtype, SUNDenseMatrix, SUNDenseMatrix_Cols, SUNDenseMatrix_Columns, SUNDenseMatrix_Rows,
    SUNMatClone, SUNMatCopy, SUNMatDestroy, SUNMatMatvec, SUNMatMatvecSetup, SUNMatScaleAdd,
    SUNMatZero, SUNMatrix,
};

use crate::{
    ode_solver::sundials::sundials_check,
    op::NonLinearOp,
    scalar::scale,
    vector::sundials::{get_suncontext, SundialsVector},
    IndexType, Scale, SundialsLinearSolver, Vector,
};

use super::{default_solver::DefaultSolver, Dense, Matrix, MatrixCommon, MatrixSparsity};
use anyhow::anyhow;

#[derive(Debug)]
pub struct SundialsMatrix {
    sm: SUNMatrix,
    owned: bool,
}

impl SundialsMatrix {
    pub fn new_dense(m: IndexType, n: IndexType) -> Self {
        let ctx = get_suncontext();
        let sm = unsafe { SUNDenseMatrix(m.try_into().unwrap(), n.try_into().unwrap(), *ctx) };
        SundialsMatrix { sm, owned: true }
    }
    pub fn new_not_owned(sm: SUNMatrix) -> Self {
        SundialsMatrix { sm, owned: false }
    }
    pub fn new_clone(v: &Self) -> Self {
        let sm = unsafe { SUNMatClone(v.sm) };
        SundialsMatrix { sm, owned: true }
    }
    pub fn sundials_matrix(&self) -> SUNMatrix {
        self.sm
    }

    fn map_inplace<F>(&mut self, f: F)
    where
        F: Fn(realtype) -> realtype,
    {
        let n = self.ncols();
        let m = self.nrows();
        for i in 0..m {
            for j in 0..n {
                self[(i, j)] = f(self[(i, j)]);
            }
        }
    }
}

impl Drop for SundialsMatrix {
    fn drop(&mut self) {
        if self.owned {
            unsafe { SUNMatDestroy(self.sm) };
        }
    }
}

impl Display for SundialsMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.ncols();
        let m = self.nrows();
        for i in 0..m {
            for j in 0..n {
                write!(f, "{} ", self[(i, j)])?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

impl DefaultSolver for SundialsMatrix {
    type LS<C: NonLinearOp<M = SundialsMatrix, V = SundialsVector, T = realtype>> =
        SundialsLinearSolver<C>;
}

impl MatrixCommon for SundialsMatrix {
    type V = SundialsVector;
    type T = realtype;

    fn nrows(&self) -> crate::IndexType {
        unsafe { SUNDenseMatrix_Rows(self.sm).try_into().unwrap() }
    }

    fn ncols(&self) -> crate::IndexType {
        unsafe { SUNDenseMatrix_Columns(self.sm).try_into().unwrap() }
    }
}

// index
impl Index<(IndexType, IndexType)> for SundialsMatrix {
    type Output = realtype;

    fn index(&self, index: (IndexType, IndexType)) -> &Self::Output {
        let (i, j) = index;
        let n = self.ncols();
        let m = self.nrows();
        if i >= m || j >= n {
            panic!("Index out of bounds");
        }
        unsafe { &*(*SUNDenseMatrix_Cols(self.sm).add(j)).add(i) }
    }
}

// index_mut
impl IndexMut<(IndexType, IndexType)> for SundialsMatrix {
    fn index_mut(&mut self, index: (IndexType, IndexType)) -> &mut Self::Output {
        let (i, j) = index;
        let n = self.ncols();
        let m = self.nrows();
        if i >= m || j >= n {
            panic!("Index out of bounds");
        }
        unsafe { &mut *(*SUNDenseMatrix_Cols(self.sm).add(j)).add(i) }
    }
}

// clone
impl Clone for SundialsMatrix {
    fn clone(&self) -> Self {
        let ret = SundialsMatrix::new_clone(self);
        sundials_check(unsafe { SUNMatCopy(self.sm, ret.sm) }).unwrap();
        ret
    }
}

// add assign and subtract assign
impl AddAssign<&SundialsMatrix> for SundialsMatrix {
    fn add_assign(&mut self, rhs: &SundialsMatrix) {
        sundials_check(unsafe { SUNMatScaleAdd(1.0, self.sm, rhs.sm) }).unwrap();
    }
}

impl SubAssign<&SundialsMatrix> for SundialsMatrix {
    fn sub_assign(&mut self, rhs: &SundialsMatrix) {
        sundials_check(unsafe { SUNMatScaleAdd(-1.0, self.sm, rhs.sm) }).unwrap();
        self.mul_assign(scale(-1.0));
    }
}

// add and subtract
// create a macro for both add and subtract
macro_rules! impl_bin_op {
    ($trait:ident, $fn:ident, $op:tt) => {
        impl $trait<&SundialsMatrix> for SundialsMatrix {
            type Output = SundialsMatrix;

            fn $fn(mut self, rhs: &SundialsMatrix) -> Self::Output {
                self $op rhs;
                self
            }
        }

        impl $trait<SundialsMatrix> for SundialsMatrix {
            type Output = SundialsMatrix;

            fn $fn(mut self, rhs: SundialsMatrix) -> Self::Output {
                self $op &rhs;
                self
            }
        }

        impl $trait<&SundialsMatrix> for &SundialsMatrix {
            type Output = SundialsMatrix;

            fn $fn(self, rhs: &SundialsMatrix) -> Self::Output {
                let mut m = self.clone();
                m $op rhs;
                m
            }
        }
    };
}

impl_bin_op!(Add, add, +=);
impl_bin_op!(Sub, sub, -=);

// mul and div by scalar
macro_rules! impl_scalar_op {
    ($trait:ident, $fn:ident, $op:tt) => {
        impl $trait<Scale<realtype>> for SundialsMatrix {
            type Output = SundialsMatrix;

            fn $fn(mut self, rhs: Scale<realtype>) -> Self::Output {
                self.map_inplace(|x| x $op rhs.value());
                self
            }
        }

        impl $trait<Scale<realtype>> for &SundialsMatrix {
            type Output = SundialsMatrix;

            fn $fn(self, rhs: Scale<realtype>) -> Self::Output {
                let mut m = self.clone();
                m.map_inplace(|x| x $op rhs.value());
                m
            }
        }
    };
}

impl_scalar_op!(Mul, mul, *);
impl_scalar_op!(Div, div, /);

macro_rules! impl_scalar_assign_op {
    ($trait:ident, $fn:ident, $op:tt) => {
        impl $trait<Scale<realtype>> for SundialsMatrix {
            fn $fn(&mut self, rhs: Scale<realtype>) {
                self.map_inplace(|x| x $op rhs.value());
            }
        }
    }
}

impl_scalar_assign_op!(MulAssign, mul_assign, *);
impl_scalar_assign_op!(DivAssign, div_assign, /);

impl Matrix for SundialsMatrix {
    type Sparsity = Dense;

    fn set_data_with_indices(
        &mut self,
        dst_indices: &<Self::Sparsity as MatrixSparsity>::Index,
        src_indices: &<Self::V as Vector>::Index,
        data: &Self::V,
    ) {
        for ((i, j), src_i) in dst_indices.iter().zip(src_indices.iter()) {
            self[(*i, *j)] = data[*src_i];
        }
    }

    fn diagonal(&self) -> Self::V {
        let n = min(self.nrows(), self.ncols());
        let mut v = SundialsVector::new_serial(n);
        for i in 0..n {
            v[i] = self[(i, i)];
        }
        v
    }

    fn copy_from(&mut self, other: &Self) {
        let ret = unsafe { SUNMatCopy(other.sm, self.sm) };
        if ret != 0 {
            panic!("Error copying matrix");
        }
    }

    fn set_column(&mut self, j: IndexType, v: &Self::V) {
        let n = self.nrows();
        if v.len() != n {
            panic!("Vector length does not match matrix size");
        }
        for i in 0..n {
            self[(i, j)] = v[i];
        }
    }

    fn scale_add_and_assign(&mut self, x: &Self, beta: Self::T, y: &Self) {
        self.copy_from(y);
        sundials_check(unsafe { SUNMatScaleAdd(beta, self.sm, x.sm) }).unwrap();
    }

    fn zeros(nrows: IndexType, ncols: IndexType) -> Self {
        let m = SundialsMatrix::new_dense(nrows, ncols);
        unsafe { SUNMatZero(m.sm) };
        m
    }

    fn from_diagonal(v: &Self::V) -> Self {
        let n = v.len();
        let mut m = SundialsMatrix::new_dense(n, n);
        for i in 0..n {
            m[(i, i)] = v[i];
        }
        m
    }

    fn try_from_triplets(
        nrows: crate::IndexType,
        ncols: crate::IndexType,
        triplets: Vec<(crate::IndexType, crate::IndexType, Self::T)>,
    ) -> anyhow::Result<Self> {
        let mut m = Self::zeros(nrows, ncols);
        for (i, j, val) in triplets {
            if i >= nrows || j >= ncols {
                return Err(anyhow!("Index out of bounds"));
            }
            m[(i, j)] = val;
        }
        Ok(m)
    }

    /// Perform a matrix-vector multiplication `y = alpha * self * x + beta * y`.
    fn gemv(&self, alpha: Self::T, x: &Self::V, beta: Self::T, y: &mut Self::V) {
        let a = self.sundials_matrix();
        let tmp = SundialsVector::new_serial(self.nrows());
        sundials_check(unsafe { SUNMatMatvecSetup(a) }).unwrap();
        sundials_check(unsafe { SUNMatMatvec(a, x.sundials_vector(), tmp.sundials_vector()) })
            .unwrap();
        y.axpy(alpha, &tmp, beta);
    }

    fn new_from_sparsity(
        nrows: IndexType,
        ncols: IndexType,
        _sparsity: Option<&Self::Sparsity>,
    ) -> Self {
        Self::new_dense(nrows, ncols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexing() {
        let mut m = SundialsMatrix::new_dense(2, 2);
        m[(0, 0)] = 1.0;
        m[(0, 1)] = 2.0;
        m[(1, 0)] = 3.0;
        m[(1, 1)] = 4.0;
        assert_eq!(m[(0, 0)], 1.0);
        assert_eq!(m[(0, 1)], 2.0);
        assert_eq!(m[(1, 0)], 3.0);
        assert_eq!(m[(1, 1)], 4.0);
    }

    #[test]
    fn test_add_sub() {
        let mut m1 = SundialsMatrix::new_dense(2, 2);
        m1[(0, 0)] = 1.0;
        m1[(0, 1)] = 2.0;
        m1[(1, 0)] = 3.0;
        m1[(1, 1)] = 4.0;

        let mut m2 = SundialsMatrix::new_dense(2, 2);
        m2[(0, 0)] = 1.0;
        m2[(0, 1)] = 2.0;
        m2[(1, 0)] = 3.0;
        m2[(1, 1)] = 4.0;

        let m3 = &m1 + &m2;
        assert_eq!(m3[(0, 0)], 2.0);
        assert_eq!(m3[(0, 1)], 4.0);
        assert_eq!(m3[(1, 0)], 6.0);
        assert_eq!(m3[(1, 1)], 8.0);

        let m4 = &m1 - &m2;
        assert_eq!(m4[(0, 0)], 0.0);
        assert_eq!(m4[(0, 1)], 0.0);
        assert_eq!(m4[(1, 0)], 0.0);
        assert_eq!(m4[(1, 1)], 0.0);

        let m5 = m1 + &m2;
        assert_eq!(m5[(0, 0)], 2.0);
        assert_eq!(m5[(0, 1)], 4.0);
        assert_eq!(m5[(1, 0)], 6.0);
        assert_eq!(m5[(1, 1)], 8.0);
    }

    #[test]
    fn test_mul_div_scalar() {
        let mut m = SundialsMatrix::new_dense(2, 2);
        m[(0, 0)] = 1.0;
        m[(0, 1)] = 2.0;
        m[(1, 0)] = 3.0;
        m[(1, 1)] = 4.0;

        let m2 = &m * scale(2.0);
        assert_eq!(m2[(0, 0)], 2.0);
        assert_eq!(m2[(0, 1)], 4.0);
        assert_eq!(m2[(1, 0)], 6.0);
        assert_eq!(m2[(1, 1)], 8.0);

        let m3 = &m / scale(2.0);
        assert_eq!(m3[(0, 0)], 0.5);
        assert_eq!(m3[(0, 1)], 1.0);
        assert_eq!(m3[(1, 0)], 1.5);
        assert_eq!(m3[(1, 1)], 2.0);

        let m4 = m * scale(2.0);
        assert_eq!(m4[(0, 0)], 2.0);
        assert_eq!(m4[(0, 1)], 4.0);
        assert_eq!(m4[(1, 0)], 6.0);
        assert_eq!(m4[(1, 1)], 8.0);
    }

    #[test]
    fn test_try_from_triplets() {
        let m = SundialsMatrix::try_from_triplets(2, 2, vec![(0, 0, 1.0), (1, 1, 2.0)]).unwrap();
        assert_eq!(m[(0, 0)], 1.0);
        assert_eq!(m[(0, 1)], 0.0);
        assert_eq!(m[(1, 0)], 0.0);
        assert_eq!(m[(1, 1)], 2.0);
    }
}
