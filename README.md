This is a personal project that I'm sharing for others to enjoy and improve. It's a particle simulation written in rust, rendered with the macroquad library. It follows a set of very simple rules to create interesting emergent behavior. 
  
Each particle has a type, and each type has a different attraction value to each other type. Every simulation step, particles that are near enough to each other apply acceleration towards each other, with each particle's acceleration being proportional to its attraction value, and inversely proportional to their distance. When particles overlap, they in stead have a strong repulsive force. Equal and opposite forces aren't guarenteed (and are in fact quite rare outside of particles of the same type), which results in many glider-like patterns emerging. Due to the lack of conservation of energy this results in, a steep drag coeficient is applied to each particle as well, to keep speeds managable and to give particles enough time to interact. 

The actual simulation backend uses a bucketing system to improve performance. 

Rendering performance is suboptimal due to using macroquad's built in circle drawing system. I'm looking into improving this. 

I also plan to add a UI for modifying the state of the simulation. For now, `R` can be used to reset the simulation with a random state. 

The camera can be moved with `WASD`, and zoomed with the scroll wheel. Press `C` to center it on the simulation. 

To run this program, clone the repository and compile it using cargo with release mode enabled for optimal performance. I may consider adding precompiled binaries, but there aren't any right now now. 
