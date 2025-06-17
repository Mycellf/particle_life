This is a particle simulation written in rust, rendered with the macroquad library (egui used for ui). It follows a set of simple rules to create interesting emergent behavior. 
  
Each particle has a type, and each type has a different attraction value to each other type. Every simulation step, particles that are near to each other accelerate towards or away from each other. Acceleration is proportional to a particle's attraction value, and inversely proportional to distance. When particles overlap, they have a strong repulsive acceleration. Equal and opposite forces aren't guaranteed (and are in fact quite rare outside of particles of the same type), which results in a lot of motion. Due to the lack of conservation of energy this results in, each particle also has a strong drag coefficient. 

The simulation backend uses rayon parallelized buckets to improve performance. 

The camera can be moved with `WASD`, and zoomed with the scroll wheel. Press `C` to center it on the simulation. 
