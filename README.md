This is a personal project that I'm sharing for others to enjoy and improve. It's a particle simulation written in rust, rendered with the macroquad library. It follows a set of very simple rules to create interesting emergent behavior. 
  
Each particle has a type, and each type has a different attraction value to each other type. Every simulation step, particles that are near enough to each other apply acceleration towards each other, with the acceleration being proportional to its attraction value, and inversely proportional to their distance. When particles overlap, they in stead have a strong repulsive acceleration. Equal and opposite forces aren't guaranteed (and are in fact quite rare outside of particles of the same type), which results in many glider-like patterns emerging. Due to the lack of conservation of energy this results in, a steep drag coefficient is applied to each particle as well, to keep speeds manageable and to give particles enough time to interact. 

The simulation backend uses rayon parallelized buckets to improve performance. 

I also plan to add a UI for modifying the state of the simulation. For now, `R` can be used to reset the simulation with a random state. 

The camera can be moved with `WASD`, and zoomed with the scroll wheel. Press `C` to center it on the simulation. 

To run this program, clone the repository and compile it using cargo with release mode enabled for optimal performance. I may consider adding precompiled binaries, but there aren't any right now now. 
