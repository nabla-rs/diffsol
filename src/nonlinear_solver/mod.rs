use anyhow::Result;
use core::panic;
use num_traits::{One, Pow};
use std::rc::Rc;

use crate::{op::Op, scalar::scale, solver::SolverProblem, IndexType, Scalar, Vector};

pub struct NonLinearSolveSolution<V> {
    pub x0: V,
    pub x: V,
}

impl<V> NonLinearSolveSolution<V> {
    pub fn new(x0: V, x: V) -> Self {
        Self { x0, x }
    }
}

/// A solver for the nonlinear problem `F(x) = 0`.
pub trait NonLinearSolver<C: Op> {
    /// Get the problem to be solved.
    fn problem(&self) -> &SolverProblem<C>;

    /// Set the problem to be solved, any previous problem is discarded.
    fn set_problem(&mut self, problem: &SolverProblem<C>);

    /// Reset the approximation of the Jacobian matrix.
    fn reset_jacobian(&mut self, x: &C::V, t: C::T);

    // Solve the problem `F(x, t) = 0` for fixed t, and return the solution `x`.
    fn solve(&mut self, x: &C::V, t: C::T) -> Result<C::V> {
        let mut x = x.clone();
        self.solve_in_place(&mut x, t)?;
        Ok(x)
    }

    // Solve the problem `F(x) = 0` in place.
    fn solve_in_place(&mut self, x: &mut C::V, t: C::T) -> Result<()>;

    // Set the maximum number of iterations for the solver.
    fn set_max_iter(&mut self, max_iter: usize);

    // Get the maximum number of iterations for the solver.
    fn max_iter(&self) -> usize;

    // Get the number of iterations taken by the solver on the last call to `solve`.
    fn niter(&self) -> usize;
}

struct Convergence<C: Op> {
    rtol: C::T,
    atol: Rc<C::V>,
    tol: C::T,
    max_iter: IndexType,
    iter: IndexType,
    scale: Option<C::V>,
    old_norm: Option<C::T>,
}

enum ConvergenceStatus {
    Converged,
    Diverged,
    Continue,
    MaximumIterations,
}

impl<C: Op> Convergence<C> {
    fn new(problem: &SolverProblem<C>, max_iter: IndexType) -> Self {
        let rtol = problem.rtol;
        let atol = problem.atol.clone();
        let minimum_tol = C::T::from(10.0) * C::T::EPSILON / rtol;
        let maximum_tol = C::T::from(0.03);
        let mut tol = C::T::from(0.5) * rtol.pow(C::T::from(0.5));
        if tol > maximum_tol {
            tol = maximum_tol;
        }
        if tol < minimum_tol {
            tol = minimum_tol;
        }
        Self {
            rtol,
            atol,
            tol,
            max_iter,
            scale: None,
            old_norm: None,
            iter: 0,
        }
    }
    fn reset(&mut self, y: &C::V) {
        let mut scale = y.abs() * scale(self.rtol);
        scale += self.atol.as_ref();
        self.scale = Some(scale);
        self.iter = 0;
        self.old_norm = None;
    }
    fn check_new_iteration(&mut self, dy: &mut C::V) -> ConvergenceStatus {
        if self.scale.is_none() {
            panic!("Convergence::check_new_iteration() called before Convergence::reset()");
        }
        dy.component_div_assign(self.scale.as_ref().unwrap());
        let norm = dy.norm();
        // if norm is zero then we are done
        if norm <= C::T::EPSILON {
            return ConvergenceStatus::Converged;
        }
        if let Some(old_norm) = self.old_norm {
            let rate = norm / old_norm;

            if rate > C::T::from(1.0) {
                return ConvergenceStatus::Diverged;
            }

            // if converged then break out of iteration successfully
            if rate / (C::T::one() - rate) * norm < self.tol {
                return ConvergenceStatus::Converged;
            }

            // if iteration is not going to converge in NEWTON_MAXITER
            // (assuming the current rate), then abort
            if rate.pow(i32::try_from(self.max_iter - self.iter).unwrap())
                / (C::T::from(1.0) - rate)
                * norm
                > self.tol
            {
                return ConvergenceStatus::Diverged;
            }
        }
        self.iter += 1;
        self.old_norm = Some(norm);
        if self.iter >= self.max_iter {
            ConvergenceStatus::MaximumIterations
        } else {
            ConvergenceStatus::Continue
        }
    }
}

pub mod newton;

//tests
#[cfg(test)]
pub mod tests {
    use self::newton::NewtonNonlinearSolver;
    use crate::{
        linear_solver::nalgebra::lu::LU,
        matrix::MatrixCommon,
        op::{closure::Closure, NonLinearOp},
        DenseMatrix,
    };

    use super::*;
    use num_traits::Zero;

    pub fn get_square_problem<M>() -> (
        SolverProblem<impl NonLinearOp<M = M, V = M::V, T = M::T>>,
        Vec<NonLinearSolveSolution<M::V>>,
    )
    where
        M: DenseMatrix + 'static,
    {
        let jac1 = M::from_diagonal(&M::V::from_vec(vec![2.0.into(), 2.0.into()]));
        let jac2 = jac1.clone();
        let p = Rc::new(M::V::zeros(0));
        let op = Closure::new(
            // 0 = J * x * x - 8
            move |x: &<M as MatrixCommon>::V, _p: &<M as MatrixCommon>::V, _t, y| {
                jac1.gemv(M::T::one(), x, M::T::zero(), y); // y = J * x
                y.component_mul_assign(x);
                y.add_scalar_mut(M::T::from(-8.0));
            },
            // J = 2 * J * x * dx
            move |x: &<M as MatrixCommon>::V, _p: &<M as MatrixCommon>::V, _t, v, y| {
                jac2.gemv(M::T::from(2.0), x, M::T::zero(), y); // y = 2 * J * x
                y.component_mul_assign(v);
            },
            2,
            2,
            p,
        );
        let rtol = M::T::from(1e-6);
        let atol = M::V::from_vec(vec![1e-6.into(), 1e-6.into()]);
        let problem = SolverProblem::new(Rc::new(op), Rc::new(atol), rtol);
        let solns = vec![NonLinearSolveSolution::new(
            M::V::from_vec(vec![2.1.into(), 2.1.into()]),
            M::V::from_vec(vec![2.0.into(), 2.0.into()]),
        )];
        (problem, solns)
    }

    pub fn test_nonlinear_solver<C>(
        mut solver: impl NonLinearSolver<C>,
        problem: SolverProblem<C>,
        solns: Vec<NonLinearSolveSolution<C::V>>,
    ) where
        C: NonLinearOp,
    {
        solver.set_problem(&problem);
        let t = C::T::zero();
        for soln in solns {
            let x = solver.solve(&soln.x0, t).unwrap();
            let tol = { soln.x.abs() * scale(problem.rtol) + problem.atol.as_ref() };
            x.assert_eq(&soln.x, &tol);
        }
    }

    type MCpu = nalgebra::DMatrix<f64>;

    #[test]
    fn test_newton_cpu_square() {
        let lu = LU::default();
        let (prob, soln) = get_square_problem::<MCpu>();
        let s = NewtonNonlinearSolver::new(lu);
        test_nonlinear_solver(s, prob, soln);
    }
}
