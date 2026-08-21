FROM gcc:14
WORKDIR /src
COPY src/ /src/
RUN g++ -std=c++23 -O3 -Wall -Wextra -Wpedantic -Werror staged_deploy.cpp -pthread -o /usr/local/bin/drm_staged_deploy
ENTRYPOINT ["/usr/local/bin/drm_staged_deploy"]
CMD ["--out","/results"]
