use std::ops::{Index, IndexMut};

#[derive(Clone, Debug)]
pub struct Matrix<T> {
    pub size: [usize; 2],
    pub data: Box<[T]>,
}

impl<T> Matrix<T> {
    pub fn from_element(size: [usize; 2], element: T) -> Self
    where
        T: Clone,
    {
        Self {
            size,
            data: std::iter::repeat(element).take(size[0] * size[1]).collect(),
        }
    }

    pub fn from_fn<F>(size: [usize; 2], function: F) -> Self
    where
        F: FnMut([usize; 2]) -> T,
    {
        let data = (0..size[0])
            .flat_map(|i| (0..size[1]).map(move |j| [i, j]))
            .map(function)
            .collect();
        Self { size, data }
    }

    pub fn get(&self, index: [usize; 2]) -> Option<&T> {
        self.check_index_bounds(index)?;
        Some(&self.data[index[0] + index[1] * self.size[0]])
    }

    pub fn get_mut(&mut self, index: [usize; 2]) -> Option<&mut T> {
        self.check_index_bounds(index)?;
        Some(&mut self.data[index[0] + index[1] * self.size[0]])
    }

    pub fn check_index_bounds(&self, index: [usize; 2]) -> Option<()> {
        if index[0] < self.size[0] && index[1] < self.size[1] {
            Some(())
        } else {
            None
        }
    }
}

impl<T> Index<[usize; 2]> for Matrix<T> {
    type Output = T;

    fn index(&self, index: [usize; 2]) -> &Self::Output {
        self.get(index).unwrap()
    }
}

impl<T> IndexMut<[usize; 2]> for Matrix<T> {
    fn index_mut(&mut self, index: [usize; 2]) -> &mut Self::Output {
        self.get_mut(index).unwrap()
    }
}
